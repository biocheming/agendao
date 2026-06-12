use super::session_actions::TranscriptOptions;
use super::*;
use crate::api::SkillEvolutionProposalKind;
use crate::components::ProviderConnectMode;
use crate::context::DialogSlot;

impl App {
    pub(super) fn open_alert_dialog(&mut self) {
        self.alert_dialog.open();
        self.context.sync_dialog_open(DialogSlot::Alert, true);
    }

    pub(super) fn close_alert_dialog(&mut self) {
        self.alert_dialog.close();
        self.context.close_dialog(DialogSlot::Alert);
    }

    pub(super) fn open_help_dialog(&mut self) {
        self.help_dialog.open();
        self.context.sync_dialog_open(DialogSlot::Help, true);
    }

    pub(super) fn close_help_dialog(&mut self) {
        self.help_dialog.close();
        self.context.close_dialog(DialogSlot::Help);
    }

    pub(super) fn open_command_palette_dialog(&mut self) {
        self.command_palette.open();
        self.context
            .sync_dialog_open(DialogSlot::CommandPalette, true);
    }

    pub(super) fn close_command_palette_dialog(&mut self) {
        self.command_palette.close();
        self.context.close_dialog(DialogSlot::CommandPalette);
    }

    pub(super) fn open_model_select_dialog(&mut self) {
        self.model_select.open();
        self.context.sync_dialog_open(DialogSlot::ModelSelect, true);
    }

    pub(super) fn close_model_select_dialog(&mut self) {
        self.model_select.close();
        self.context.close_dialog(DialogSlot::ModelSelect);
    }

    pub(super) fn open_agent_select_dialog(&mut self) {
        self.agent_select.open();
        self.context.sync_dialog_open(DialogSlot::AgentSelect, true);
    }

    pub(super) fn close_agent_select_dialog(&mut self) {
        self.agent_select.close();
        self.context.close_dialog(DialogSlot::AgentSelect);
    }

    pub(super) fn open_status_dialog_modal(&mut self) {
        self.status_dialog.open();
        self.context.sync_dialog_open(DialogSlot::Status, true);
    }

    pub(super) fn close_status_dialog_modal(&mut self) {
        self.status_dialog.close();
        self.context.close_dialog(DialogSlot::Status);
    }

    pub(super) fn open_session_rename_dialog_modal(&mut self, session_id: String, title: String) {
        self.session_rename_dialog.open(session_id, title);
        self.context
            .sync_dialog_open(DialogSlot::SessionRename, true);
    }

    pub(super) fn close_session_rename_dialog_modal(&mut self) {
        self.session_rename_dialog.close();
        self.context.close_dialog(DialogSlot::SessionRename);
    }

    pub(super) fn open_session_export_dialog_modal(
        &mut self,
        session_id: String,
        default_filename: String,
    ) {
        self.session_export_dialog
            .open(session_id, default_filename);
        self.context
            .sync_dialog_open(DialogSlot::SessionExport, true);
    }

    pub(super) fn close_session_export_dialog_modal(&mut self) {
        self.session_export_dialog.close();
        self.context.close_dialog(DialogSlot::SessionExport);
    }

    pub(super) fn open_recovery_action_dialog_modal(&mut self, items: Vec<RecoveryActionItem>) {
        self.recovery_action_dialog.open(items);
        self.context
            .sync_dialog_open(DialogSlot::RecoveryAction, true);
    }

    pub(super) fn close_recovery_action_dialog_modal(&mut self) {
        self.recovery_action_dialog.close();
        self.context.close_dialog(DialogSlot::RecoveryAction);
    }

    pub(super) fn open_prompt_stash_dialog_modal(&mut self) {
        self.prompt_stash_dialog.open();
        self.context.sync_dialog_open(DialogSlot::PromptStash, true);
    }

    pub(super) fn close_prompt_stash_dialog_modal(&mut self) {
        self.prompt_stash_dialog.close();
        self.context.close_dialog(DialogSlot::PromptStash);
    }

    pub(super) fn open_skill_list_dialog_modal(&mut self) {
        self.skill_list_dialog.open();
        self.context.sync_dialog_open(DialogSlot::SkillList, true);
    }

    pub(super) fn close_skill_list_dialog_modal(&mut self) {
        self.skill_list_dialog.close();
        self.context.close_dialog(DialogSlot::SkillList);
    }

    pub(super) fn open_session_list_dialog_modal(&mut self, current_session_id: Option<&str>) {
        self.session_list_dialog.open(current_session_id);
        self.context.sync_dialog_open(DialogSlot::SessionList, true);
    }

    pub(super) fn close_session_list_dialog_modal(&mut self) {
        self.session_list_dialog.close();
        self.context.close_dialog(DialogSlot::SessionList);
    }

    pub(super) fn open_theme_list_dialog_modal(&mut self, current_theme: &str) {
        self.theme_list_dialog.open(current_theme);
        self.context.sync_dialog_open(DialogSlot::ThemeList, true);
    }

    pub(super) fn close_theme_list_dialog_modal(&mut self) {
        let initial = self.theme_list_dialog.initial_theme_id().to_string();
        let _ = self.context.set_theme_by_name(&initial);
        self.theme_list_dialog.close();
        self.context.close_dialog(DialogSlot::ThemeList);
    }

    pub(super) fn open_mcp_dialog_modal(&mut self) {
        self.mcp_dialog.open();
        self.context.sync_dialog_open(DialogSlot::Mcp, true);
    }

    pub(super) fn close_mcp_dialog_modal(&mut self) {
        self.mcp_dialog.close();
        self.context.close_dialog(DialogSlot::Mcp);
    }

    pub(super) fn open_timeline_dialog_modal(&mut self, entries: Vec<TimelineEntry>) {
        self.timeline_dialog.open(entries);
        self.context.sync_dialog_open(DialogSlot::Timeline, true);
    }

    pub(super) fn close_timeline_dialog_modal(&mut self) {
        self.timeline_dialog.close();
        self.context.close_dialog(DialogSlot::Timeline);
    }

    pub(super) fn open_fork_dialog_modal(&mut self, session_id: String, entries: Vec<ForkEntry>) {
        self.fork_dialog.open(session_id, entries);
        self.context.sync_dialog_open(DialogSlot::Fork, true);
    }

    pub(super) fn close_fork_dialog_modal(&mut self) {
        self.fork_dialog.close();
        self.context.close_dialog(DialogSlot::Fork);
    }

    pub(super) fn open_provider_dialog_modal(&mut self) {
        self.provider_dialog.open();
        self.context.sync_dialog_open(DialogSlot::Provider, true);
    }

    pub(super) fn close_provider_dialog_modal(&mut self) {
        self.provider_dialog.close();
        self.context.close_dialog(DialogSlot::Provider);
    }

    pub(super) fn close_subagent_dialog_modal(&mut self) {
        self.subagent_dialog.close();
        self.context.close_dialog(DialogSlot::Subagent);
    }

    pub(super) fn close_tag_dialog_modal(&mut self) {
        self.tag_dialog.close();
        self.context.close_dialog(DialogSlot::Tag);
    }

    pub(super) fn open_skill_proposal_review_dialog(&mut self) {
        let client = match self.context.get_api_client() {
            Some(c) => c,
            None => return,
        };
        let proposals = match client.list_skill_proposals("draft") {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to fetch proposals: {}", e);
                return;
            }
        };
        if proposals.is_empty() {
            return;
        }
        let items: Vec<SkillProposalReviewItem> = proposals
            .into_iter()
            .map(|p| {
                let kind_label = match p.proposal_kind {
                    SkillEvolutionProposalKind::PatchExistingSkill => "Patch",
                    SkillEvolutionProposalKind::CreateNewSkill => "Create",
                };
                let skill_name = p
                    .linked_skill_name
                    .as_deref()
                    .unwrap_or("(new skill)")
                    .to_string();
                let first_change = p
                    .suggested_changes
                    .first()
                    .map(change_summary)
                    .unwrap_or_default();
                SkillProposalReviewItem {
                    id: p.id,
                    title: p.title,
                    kind_label: kind_label.to_string(),
                    skill_name,
                    first_change,
                }
            })
            .collect();
        self.skill_proposal_review_dialog.open(items);
    }

    pub(super) fn close_skill_proposal_review_dialog(&mut self) {
        self.skill_proposal_review_dialog.close();
    }

    pub(super) fn apply_skill_proposal_action(&mut self, id: &str, status: &str) {
        let client = match self.context.get_api_client() {
            Some(c) => c,
            None => {
                self.skill_proposal_review_dialog
                    .set_status("No server connection.");
                return;
            }
        };
        match client.update_skill_proposal_status(id, status) {
            Ok(_) => {
                self.skill_proposal_review_dialog.remove_current();
                self.skill_proposal_review_dialog
                    .set_status(&format!("Proposal {} {}.", id, status));
                self.toast
                    .show(ToastVariant::Success, "Proposal updated.", 2000);
            }
            Err(e) => {
                self.skill_proposal_review_dialog
                    .set_status(&format!("Failed: {}", e));
            }
        }
    }

    pub(super) fn open_tool_call_cancel_dialog_modal(&mut self, items: Vec<ToolCallItem>) {
        self.tool_call_cancel_dialog.open(items);
        self.context
            .sync_dialog_open(DialogSlot::ToolCallCancel, true);
    }

    pub(super) fn close_tool_call_cancel_dialog_modal(&mut self) {
        self.tool_call_cancel_dialog.close();
        self.context.close_dialog(DialogSlot::ToolCallCancel);
    }

    pub(super) fn close_slash_popup_dialog(&mut self) {
        self.slash_popup.close();
        self.context.close_dialog(DialogSlot::SlashPopup);
    }

    fn sync_dialog_lifecycle(&self) {
        self.context
            .sync_dialog_open(DialogSlot::Alert, self.alert_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::Help, self.help_dialog.is_open());
        self.context.sync_dialog_open(
            DialogSlot::RecoveryAction,
            self.recovery_action_dialog.is_open(),
        );
        self.context
            .sync_dialog_open(DialogSlot::Status, self.status_dialog.is_open());
        self.context.sync_dialog_open(
            DialogSlot::SessionRename,
            self.session_rename_dialog.is_open(),
        );
        self.context.sync_dialog_open(
            DialogSlot::SessionExport,
            self.session_export_dialog.is_open(),
        );
        self.context
            .sync_dialog_open(DialogSlot::PromptStash, self.prompt_stash_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::SkillList, self.skill_list_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::SlashPopup, self.slash_popup.is_open());
        self.context
            .sync_dialog_open(DialogSlot::CommandPalette, self.command_palette.is_open());
        self.context
            .sync_dialog_open(DialogSlot::ModelSelect, self.model_select.is_open());
        self.context
            .sync_dialog_open(DialogSlot::AgentSelect, self.agent_select.is_open());
        self.context
            .sync_dialog_open(DialogSlot::SessionList, self.session_list_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::ThemeList, self.theme_list_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::Mcp, self.mcp_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::Timeline, self.timeline_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::Fork, self.fork_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::Provider, self.provider_dialog.is_open());
        self.context
            .sync_dialog_open(DialogSlot::Subagent, self.subagent_dialog.is_open());
        self.context.sync_dialog_open(
            DialogSlot::ToolCallCancel,
            self.tool_call_cancel_dialog.is_open(),
        );
        self.context
            .sync_dialog_open(DialogSlot::Tag, self.tag_dialog.is_open());
    }

    pub(super) fn has_reactive_home_dialog_layer(&self) -> bool {
        self.sync_dialog_lifecycle();
        self.context.has_open_dialogs()
    }

    pub(super) fn has_non_reactive_dialog_layer(&self) -> bool {
        false
    }

    pub(super) fn has_open_dialog_layer(&self) -> bool {
        self.has_reactive_home_dialog_layer() || self.has_non_reactive_dialog_layer()
    }

    pub(super) fn scroll_active_dialog(&mut self, up: bool) {
        self.sync_dialog_lifecycle();
        match self.context.top_scroll_dialog() {
            Some(DialogSlot::PromptStash) => {
                if up {
                    self.prompt_stash_dialog.move_up();
                } else {
                    self.prompt_stash_dialog.move_down();
                }
            }
            Some(DialogSlot::SkillList) => {
                if self.skill_list_dialog.is_create_mode() || self.skill_list_dialog.is_edit_mode()
                {
                    if up {
                        self.skill_list_dialog.handle_manage_page_up();
                    } else {
                        self.skill_list_dialog.handle_manage_page_down();
                    }
                } else {
                    if up {
                        self.skill_list_dialog.move_up();
                    } else {
                        self.skill_list_dialog.move_down();
                    }
                    let _ = self.refresh_skill_list_detail();
                }
            }
            Some(DialogSlot::SlashPopup) => {
                if up {
                    self.slash_popup.move_up();
                } else {
                    self.slash_popup.move_down();
                }
            }
            Some(DialogSlot::CommandPalette) => {
                if up {
                    self.command_palette.move_up();
                } else {
                    self.command_palette.move_down();
                }
            }
            Some(DialogSlot::ModelSelect) => {
                if up {
                    self.model_select.move_up();
                } else {
                    self.model_select.move_down();
                }
            }
            Some(DialogSlot::AgentSelect) => {
                if up {
                    self.agent_select.move_up();
                } else {
                    self.agent_select.move_down();
                }
            }
            Some(DialogSlot::SessionList) => {
                if self.session_list_dialog.is_renaming() {
                    return;
                }
                if up {
                    self.session_list_dialog.move_up();
                } else {
                    self.session_list_dialog.move_down();
                }
            }
            Some(DialogSlot::ThemeList) => {
                if up {
                    self.theme_list_dialog.move_up();
                } else {
                    self.theme_list_dialog.move_down();
                }
                if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                    let _ = self.context.set_theme_by_name(&theme_id);
                }
            }
            Some(DialogSlot::Mcp) => {
                if up {
                    self.mcp_dialog.move_up();
                } else {
                    self.mcp_dialog.move_down();
                }
            }
            Some(DialogSlot::Timeline) => {
                if up {
                    self.timeline_dialog.move_up();
                } else {
                    self.timeline_dialog.move_down();
                }
            }
            Some(DialogSlot::Fork) => {
                if up {
                    self.fork_dialog.move_up();
                } else {
                    self.fork_dialog.move_down();
                }
            }
            Some(DialogSlot::Provider) => {
                if up {
                    self.provider_dialog.move_up();
                } else {
                    self.provider_dialog.move_down();
                }
            }
            Some(DialogSlot::Status) => {
                if up {
                    self.status_dialog.scroll_up();
                } else {
                    self.status_dialog.scroll_down();
                }
            }
            Some(DialogSlot::Subagent) => {
                if up {
                    self.subagent_dialog.scroll_up();
                } else {
                    self.subagent_dialog.scroll_down(50);
                }
            }
            Some(DialogSlot::Tag) => {
                if up {
                    self.tag_dialog.move_up();
                } else {
                    self.tag_dialog.move_down();
                }
            }
            _ => {}
        }
    }

    pub(super) fn handle_dialog_mouse(
        &mut self,
        mouse_event: &crossterm::event::MouseEvent,
    ) -> anyhow::Result<bool> {
        use crossterm::event::{MouseButton, MouseEventKind};

        if !self.has_open_dialog_layer() {
            return Ok(false);
        }

        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_active_dialog(true);
                self.event_caused_change = true;
                Ok(true)
            }
            MouseEventKind::ScrollDown => {
                self.scroll_active_dialog(false);
                self.event_caused_change = true;
                Ok(true)
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.event_caused_change = false;
                Ok(true)
            }
            MouseEventKind::Moved
            | MouseEventKind::Down(_)
            | MouseEventKind::Drag(_)
            | MouseEventKind::Up(_) => {
                self.event_caused_change = false;
                Ok(true)
            }
            _ => {
                self.event_caused_change = false;
                Ok(true)
            }
        }
    }

    pub(super) fn handle_dialog_key(&mut self, key: KeyEvent) -> anyhow::Result<bool> {
        if self.alert_dialog.is_open() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.close_alert_dialog();
                }
                KeyCode::Char('c')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && !self.alert_dialog.message().is_empty() =>
                {
                    let _ = Clipboard::write_text(self.alert_dialog.message());
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.help_dialog.is_open() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                self.close_help_dialog();
            }
            return Ok(true);
        }
        if self.status_dialog.is_open() {
            self.handle_status_dialog_key(key);
            return Ok(true);
        }
        if self.session_rename_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_session_rename_dialog_modal(),
                KeyCode::Backspace => self.session_rename_dialog.handle_backspace(),
                KeyCode::Enter => {
                    if let Some((session_id, title)) = self.session_rename_dialog.confirm() {
                        if let Some(client) = self.context.get_api_client() {
                            if let Err(err) = client.update_session_title(&session_id, &title) {
                                self.alert_dialog.set_message(&format!(
                                    "Failed to rename session `{}`:\n{}",
                                    session_id, err
                                ));
                                self.open_alert_dialog();
                            } else {
                                self.refresh_session_list_dialog();
                                let _ = self.sync_session_from_server(&session_id);
                            }
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.session_rename_dialog.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.session_export_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_session_export_dialog_modal(),
                KeyCode::Backspace => self.session_export_dialog.handle_backspace(),
                KeyCode::Enter => {
                    if let Some(session_id) = self.session_export_dialog.session_id() {
                        let filename = self.session_export_dialog.filename().trim();
                        if filename.is_empty() {
                            self.alert_dialog
                                .set_message("Filename cannot be empty for export.");
                            self.open_alert_dialog();
                        } else {
                            let options = TranscriptOptions {
                                include_thinking: self.session_export_dialog.include_thinking,
                                include_tool_details: self
                                    .session_export_dialog
                                    .include_tool_details,
                                include_metadata: self.session_export_dialog.include_metadata,
                            };
                            match self.export_session_to_file(session_id, filename, options) {
                                Ok(path) => {
                                    self.alert_dialog.set_message(&format!(
                                        "Session exported to `{}`.",
                                        path.display()
                                    ));
                                    self.open_alert_dialog();
                                    self.close_session_export_dialog_modal();
                                }
                                Err(err) => {
                                    self.alert_dialog.set_message(&format!(
                                        "Failed to export session:\n{}",
                                        err
                                    ));
                                    self.open_alert_dialog();
                                }
                            }
                        }
                    }
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(session_id) = self.session_export_dialog.session_id() {
                        let options = TranscriptOptions {
                            include_thinking: self.session_export_dialog.include_thinking,
                            include_tool_details: self.session_export_dialog.include_tool_details,
                            include_metadata: self.session_export_dialog.include_metadata,
                        };
                        match self.build_session_transcript(session_id, options) {
                            Some(text) => {
                                if let Err(err) = Clipboard::write_text(&text) {
                                    self.alert_dialog.set_message(&format!(
                                        "Failed to copy transcript to clipboard:\n{}",
                                        err
                                    ));
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
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.session_export_dialog.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.tool_call_cancel_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_tool_call_cancel_dialog_modal(),
                KeyCode::Up => self.tool_call_cancel_dialog.previous(),
                KeyCode::Down => self.tool_call_cancel_dialog.next(),
                KeyCode::Enter => {
                    if let Some(tool_call_id) = self.tool_call_cancel_dialog.selected() {
                        if let Some(session_id) = self.current_session_id() {
                            if let Some(api) = self.context.get_api_client() {
                                if let Err(e) = api.cancel_tool_call(&session_id, &tool_call_id) {
                                    self.toast.show(
                                        ToastVariant::Error,
                                        &format!("Failed to cancel tool: {}", e),
                                        3000,
                                    );
                                } else {
                                    self.toast.show(
                                        ToastVariant::Info,
                                        "Tool cancellation requested",
                                        3000,
                                    );
                                }
                            }
                        }
                        self.close_tool_call_cancel_dialog_modal();
                    }
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.prompt_stash_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_prompt_stash_dialog_modal(),
                KeyCode::Up => self.prompt_stash_dialog.move_up(),
                KeyCode::Down => self.prompt_stash_dialog.move_down(),
                KeyCode::Backspace => self.prompt_stash_dialog.handle_backspace(),
                KeyCode::Enter => {
                    if let Some(index) = self.prompt_stash_dialog.selected_index() {
                        if self.prompt.load_stash(index) {
                            self.close_prompt_stash_dialog_modal();
                        }
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(index) = self.prompt_stash_dialog.selected_index() {
                        if self.prompt.remove_stash(index) {
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
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.prompt_stash_dialog.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.skill_list_dialog.is_open() {
            if self.skill_list_dialog.is_create_mode() || self.skill_list_dialog.is_edit_mode() {
                match key.code {
                    KeyCode::Esc => self.skill_list_dialog.cancel_manage_mode(),
                    KeyCode::Up => self.skill_list_dialog.handle_manage_up(),
                    KeyCode::Down => self.skill_list_dialog.handle_manage_down(),
                    KeyCode::Tab => self.skill_list_dialog.handle_manage_tab(false),
                    KeyCode::BackTab => self.skill_list_dialog.handle_manage_tab(true),
                    KeyCode::Left => self.skill_list_dialog.handle_manage_left(),
                    KeyCode::Right => self.skill_list_dialog.handle_manage_right(),
                    KeyCode::Home => self.skill_list_dialog.handle_manage_home(),
                    KeyCode::End => self.skill_list_dialog.handle_manage_end(),
                    KeyCode::PageUp => self.skill_list_dialog.handle_manage_page_up(),
                    KeyCode::PageDown => self.skill_list_dialog.handle_manage_page_down(),
                    KeyCode::Backspace => self.skill_list_dialog.handle_manage_backspace(),
                    KeyCode::Delete => self.skill_list_dialog.handle_manage_delete(),
                    KeyCode::Enter => self.skill_list_dialog.handle_manage_enter(),
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let result = if self.skill_list_dialog.is_create_mode() {
                            self.submit_skill_create()
                        } else {
                            self.submit_skill_edit()
                        };
                        match result {
                            Ok(message) => {
                                self.alert_dialog.set_message(&message);
                                self.open_alert_dialog();
                            }
                            Err(error) => {
                                self.alert_dialog
                                    .set_message(&format!("Failed to save skill:\n{}", error));
                                self.open_alert_dialog();
                            }
                        }
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        self.skill_list_dialog.handle_manage_char(c);
                    }
                    _ => {}
                }
                return Ok(true);
            }
            if self.skill_list_dialog.is_delete_confirm_mode() {
                match key.code {
                    KeyCode::Esc => self.skill_list_dialog.cancel_manage_mode(),
                    KeyCode::Enter => match self.submit_skill_delete() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog
                                .set_message(&format!("Failed to delete skill:\n{}", error));
                            self.open_alert_dialog();
                        }
                    },
                    _ => {}
                }
                return Ok(true);
            }
            match key.code {
                KeyCode::Esc => self.close_skill_list_dialog_modal(),
                KeyCode::Up => {
                    self.skill_list_dialog.move_up();
                    let _ = self.refresh_skill_list_detail();
                }
                KeyCode::Down => {
                    self.skill_list_dialog.move_down();
                    let _ = self.refresh_skill_list_detail();
                }
                KeyCode::PageUp => self.skill_list_dialog.preview_scroll_up(),
                KeyCode::PageDown => self.skill_list_dialog.preview_scroll_down(),
                KeyCode::Backspace => {
                    self.skill_list_dialog.handle_backspace();
                    let _ = self.refresh_skill_list_detail();
                }
                KeyCode::Enter => {
                    if let Some(skill) = self.skill_list_dialog.selected_skill() {
                        self.prompt.set_input(format!("/{} ", skill));
                        self.close_skill_list_dialog_modal();
                    }
                }
                KeyCode::Char('c')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.skill_list_dialog.begin_create();
                }
                KeyCode::Char('e')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Err(error) = self.skill_list_dialog.begin_edit() {
                        self.alert_dialog
                            .set_message(&format!("Cannot edit skill:\n{}", error));
                        self.open_alert_dialog();
                    }
                }
                KeyCode::Char('d')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Err(error) = self.skill_list_dialog.begin_delete() {
                        self.alert_dialog
                            .set_message(&format!("Cannot delete skill:\n{}", error));
                        self.open_alert_dialog();
                    }
                }
                KeyCode::Char('g')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_guard_run() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog
                                .set_message(&format!("Failed to run skill guard:\n{}", error));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('G')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_source_guard_run() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog
                                .set_message(&format!("Failed to run source guard:\n{}", error));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('i')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.skill_list_dialog.cycle_hub_source();
                }
                KeyCode::Char('x')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_index_refresh() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog.set_message(&format!(
                                "Failed to refresh source index:\n{}",
                                error
                            ));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('r')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.refresh_skill_hub_state() {
                        Ok(()) => {
                            self.alert_dialog.set_message("Refreshed skill hub state.");
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog.set_message(&format!(
                                "Failed to refresh skill hub state:\n{}",
                                error
                            ));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('p')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_sync_plan() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog
                                .set_message(&format!("Failed to build hub sync plan:\n{}", error));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('a')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_sync_apply() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog
                                .set_message(&format!("Failed to apply hub sync:\n{}", error));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('u')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_remote_install_plan() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog.set_message(&format!(
                                "Failed to build remote install plan:\n{}",
                                error
                            ));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('U')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_remote_install_apply() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog.set_message(&format!(
                                "Failed to apply remote install:\n{}",
                                error
                            ));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('v')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_remote_update_plan() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog.set_message(&format!(
                                "Failed to build remote update plan:\n{}",
                                error
                            ));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('V')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_remote_update_apply() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog
                                .set_message(&format!("Failed to apply remote update:\n{}", error));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('D')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_managed_detach() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog.set_message(&format!(
                                "Failed to detach managed skill:\n{}",
                                error
                            ));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('R')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.submit_skill_hub_managed_remove() {
                        Ok(message) => {
                            self.alert_dialog.set_message(&message);
                            self.open_alert_dialog();
                        }
                        Err(error) => {
                            self.alert_dialog.set_message(&format!(
                                "Failed to remove managed skill:\n{}",
                                error
                            ));
                            self.open_alert_dialog();
                        }
                    }
                }
                KeyCode::Char('t')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.skill_list_dialog.toggle_browse_pane();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.skill_list_dialog.handle_input(c);
                    let _ = self.refresh_skill_list_detail();
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.slash_popup.is_open() {
            match key.code {
                KeyCode::Esc => self.close_slash_popup_dialog(),
                KeyCode::Up => self.slash_popup.move_up(),
                KeyCode::Down => self.slash_popup.move_down(),
                KeyCode::Backspace => {
                    if !self.slash_popup.handle_backspace() {
                        self.close_slash_popup_dialog();
                    }
                }
                KeyCode::Enter => {
                    self.slash_popup.select_current();
                    if let Some(action) = self.slash_popup.take_action() {
                        self.execute_ui_action(action)?;
                    }
                }
                KeyCode::Char(' ')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    // Space means the user wants to type arguments (e.g. /model foo).
                    // Close the popup and inject "/{query} " into the prompt so they
                    // can continue typing.  On Enter the full text goes through
                    // parse_interactive_command() which supports parameters.
                    let query = self.slash_popup.query().to_string();
                    self.close_slash_popup_dialog();
                    self.prompt.set_input(format!("/{query} "));
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.slash_popup.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.recovery_action_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_recovery_action_dialog_modal(),
                KeyCode::Up => self.recovery_action_dialog.previous(),
                KeyCode::Down => self.recovery_action_dialog.next(),
                KeyCode::Enter => {
                    let selected = self.recovery_action_dialog.selected();
                    if let Some(selector) = selected.as_deref() {
                        self.handle_execute_recovery_action(selector);
                        self.close_recovery_action_dialog_modal();
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.command_palette.is_open() {
            match key.code {
                KeyCode::Esc => self.close_command_palette_dialog(),
                KeyCode::Up => self.command_palette.move_up(),
                KeyCode::Down => self.command_palette.move_down(),
                KeyCode::Backspace => self.command_palette.handle_backspace(),
                KeyCode::Enter => {
                    let action = self.command_palette.selected_action();
                    self.close_command_palette_dialog();
                    if let Some(action) = action {
                        self.execute_ui_action(action)?;
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.command_palette.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.model_select.is_open() {
            match key.code {
                KeyCode::Esc => self.close_model_select_dialog(),
                KeyCode::Up => self.model_select.move_up(),
                KeyCode::Down => self.model_select.move_down(),
                KeyCode::Backspace => self.model_select.handle_backspace(),
                KeyCode::Enter => {
                    if let Some(model) = self.model_select.selected_model() {
                        let provider = model.provider.clone();
                        let id = model.id.clone();
                        let model_ref = format!("{}/{}", provider, id);
                        self.model_select.push_recent(&provider, &id);
                        self.model_select.set_current_model(Some(model_ref.clone()));
                        self.context.save_recent_models(self.model_select.recent());
                        self.set_active_model_selection(model_ref, Some(provider));
                    }
                    self.close_model_select_dialog();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.model_select.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.agent_select.is_open() {
            match key.code {
                KeyCode::Esc => self.close_agent_select_dialog(),
                KeyCode::Up => self.agent_select.move_up(),
                KeyCode::Down => self.agent_select.move_down(),
                KeyCode::Enter => {
                    if let Some(agent) = self.agent_select.selected_agent() {
                        apply_selected_mode(&self.context, agent);
                    }
                    self.close_agent_select_dialog();
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.session_list_dialog.is_open() {
            if self.session_list_dialog.is_renaming() {
                match key.code {
                    KeyCode::Esc => self.session_list_dialog.cancel_rename(),
                    KeyCode::Backspace => self.session_list_dialog.handle_rename_backspace(),
                    KeyCode::Enter => {
                        if let Some((session_id, title)) = self.session_list_dialog.confirm_rename()
                        {
                            if let Some(client) = self.context.get_api_client() {
                                if let Err(err) = client.update_session_title(&session_id, &title) {
                                    self.alert_dialog.set_message(&format!(
                                        "Failed to rename session `{}`:\n{}",
                                        session_id, err
                                    ));
                                    self.open_alert_dialog();
                                } else {
                                    self.refresh_session_list_dialog();
                                    if self.current_session_id().as_deref()
                                        == Some(session_id.as_str())
                                    {
                                        let _ = self.sync_session_from_server(&session_id);
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        self.session_list_dialog.handle_rename_input(c);
                    }
                    _ => {}
                }
                return Ok(true);
            }

            match key.code {
                KeyCode::Esc => self.close_session_list_dialog_modal(),
                KeyCode::Up => self.session_list_dialog.move_up(),
                KeyCode::Down => self.session_list_dialog.move_down(),
                KeyCode::Backspace => {
                    self.session_list_dialog.handle_backspace();
                    self.refresh_session_list_dialog();
                }
                KeyCode::Enter => {
                    let target = self.session_list_dialog.selected_session_id();
                    self.close_session_list_dialog_modal();
                    if let Some(session_id) = target {
                        self.navigate_session_with_prompt_cleanup(session_id.clone());
                        let _ = self.sync_session_from_server(&session_id);
                        self.ensure_session_view(&session_id);
                    }
                }
                KeyCode::Char('r') if self.matches_keybind("session_rename", key) => {
                    let _ = self.session_list_dialog.start_rename_selected();
                }
                KeyCode::Char('d') if self.matches_keybind("session_delete", key) => {
                    if let Some(state) = self.session_list_dialog.trigger_delete_selected() {
                        match state {
                            SessionDeleteState::Armed(_) => {}
                            SessionDeleteState::Confirmed(session_id) => {
                                self.queue_delete_session(session_id);
                            }
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.session_list_dialog.handle_input(c);
                    self.refresh_session_list_dialog();
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.theme_list_dialog.is_open() {
            match key.code {
                KeyCode::Esc => {
                    let initial = self.theme_list_dialog.initial_theme_id().to_string();
                    let _ = self.context.set_theme_by_name(&initial);
                    self.close_theme_list_dialog_modal();
                }
                KeyCode::Up => {
                    self.theme_list_dialog.move_up();
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                }
                KeyCode::Down => {
                    self.theme_list_dialog.move_down();
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                }
                KeyCode::Backspace => {
                    self.theme_list_dialog.handle_backspace();
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                }
                KeyCode::Enter => {
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.commit_theme_by_name(&theme_id);
                    }
                    self.close_theme_list_dialog_modal();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.theme_list_dialog.handle_input(c);
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.mcp_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_mcp_dialog_modal(),
                KeyCode::Up => self.mcp_dialog.move_up(),
                KeyCode::Down => self.mcp_dialog.move_down(),
                KeyCode::Char('r') => {
                    let _ = self.refresh_mcp_dialog();
                }
                KeyCode::Char('a') => {
                    if let Some(item) = self.mcp_dialog.selected_item() {
                        if let Some(client) = self.context.get_api_client() {
                            match client.start_mcp_auth(&item.name) {
                                Ok(auth) => {
                                    self.alert_dialog.set_message(&format!(
                                        "MCP `{}` auth started:\n{}\n\nComplete OAuth, then reconnect.",
                                        item.name, auth.authorization_url
                                    ));
                                    self.open_alert_dialog();
                                }
                                Err(err) => {
                                    self.alert_dialog.set_message(&format!(
                                        "Failed to start MCP auth `{}`:\n{}",
                                        item.name, err
                                    ));
                                    self.open_alert_dialog();
                                }
                            }
                            let _ = client.authenticate_mcp(&item.name);
                            let _ = self.refresh_mcp_dialog();
                        }
                    }
                }
                KeyCode::Char('x') => {
                    if let Some(item) = self.mcp_dialog.selected_item() {
                        if let Some(client) = self.context.get_api_client() {
                            if let Err(err) = client.remove_mcp_auth(&item.name) {
                                self.alert_dialog.set_message(&format!(
                                    "Failed to clear MCP auth `{}`:\n{}",
                                    item.name, err
                                ));
                                self.open_alert_dialog();
                            }
                            let _ = self.refresh_mcp_dialog();
                        }
                    }
                }
                KeyCode::Enter => {
                    if let Some(item) = self.mcp_dialog.selected_item() {
                        if let Some(client) = self.context.get_api_client() {
                            let result = if item.status == "connected" {
                                client.disconnect_mcp(&item.name)
                            } else {
                                client.connect_mcp(&item.name)
                            };
                            if let Err(err) = result {
                                self.alert_dialog.set_message(&format!(
                                    "Failed to toggle MCP `{}`:\n{}",
                                    item.name, err
                                ));
                                self.open_alert_dialog();
                            }
                            let _ = self.refresh_mcp_dialog();
                        }
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.timeline_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_timeline_dialog_modal(),
                KeyCode::Up => self.timeline_dialog.move_up(),
                KeyCode::Down => self.timeline_dialog.move_down(),
                KeyCode::Enter => {
                    if let Some(msg_id) = self.timeline_dialog.selected_message_id() {
                        let msg_id = msg_id.to_string();
                        self.close_timeline_dialog_modal();
                        if let Some(sv) = self.context.session_view_handle() {
                            sv.scroll_to_message(&self.context, &msg_id);
                        }
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.fork_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_fork_dialog_modal(),
                KeyCode::Up => self.fork_dialog.move_up(),
                KeyCode::Down => self.fork_dialog.move_down(),
                KeyCode::Enter => {
                    let session_id = self.fork_dialog.session_id().map(|s| s.to_string());
                    let msg_id = self
                        .fork_dialog
                        .selected_message_id()
                        .map(|s| s.to_string());
                    self.close_fork_dialog_modal();
                    if let Some(sid) = session_id {
                        if let Some(client) = self.context.get_api_client() {
                            match client.fork_session(&sid, msg_id.as_deref()) {
                                Ok(new_session) => {
                                    self.cache_session_from_api(&new_session);
                                    self.navigate_session_with_prompt_cleanup(
                                        new_session.id.clone(),
                                    );
                                    self.ensure_session_view(&new_session.id);
                                    let _ = self.sync_session_from_server(&new_session.id);
                                    self.alert_dialog.set_message(&format!(
                                        "Forked session created: {}",
                                        new_session.title
                                    ));
                                    self.open_alert_dialog();
                                }
                                Err(err) => {
                                    self.alert_dialog
                                        .set_message(&format!("Failed to fork session:\n{}", err));
                                    self.open_alert_dialog();
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.provider_dialog.is_open() {
            if self.provider_dialog.model_override_state.is_some() {
                match key.code {
                    KeyCode::Esc => {
                        self.provider_dialog.back_model_override_flow();
                    }
                    KeyCode::Backspace => {
                        self.provider_dialog.pop_char();
                    }
                    KeyCode::Enter => {
                        if self.provider_dialog.is_model_override_final_step() {
                            if let Some(PendingSubmit::ModelOverride {
                                provider_id,
                                model_key,
                                model,
                            }) = self.provider_dialog.pending_submit()
                            {
                                self.submit_provider_model_override(
                                    &provider_id,
                                    &model_key,
                                    &model,
                                );
                            }
                        } else {
                            self.provider_dialog.advance_model_override_flow();
                        }
                    }
                    KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.paste_clipboard_to_provider_dialog();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.provider_dialog.push_char(c);
                    }
                    _ => {}
                }
            } else if self.provider_dialog.custom_state.is_some() {
                // Custom provider multi-step flow
                match key.code {
                    KeyCode::Esc => {
                        self.provider_dialog.exit_custom_flow();
                    }
                    KeyCode::Up => {
                        if self.provider_dialog.is_protocol_step() {
                            self.provider_dialog.protocol_index_dec();
                        }
                    }
                    KeyCode::Down => {
                        if self.provider_dialog.is_protocol_step() {
                            self.provider_dialog.protocol_index_inc();
                        }
                    }
                    KeyCode::Backspace => {
                        self.provider_dialog.pop_char();
                    }
                    KeyCode::Enter => {
                        if self.provider_dialog.is_final_step() {
                            // Final step - try to submit
                            if let Some(PendingSubmit::Custom {
                                provider_id,
                                base_url,
                                protocol,
                                api_key,
                            }) = self.provider_dialog.pending_submit()
                            {
                                self.submit_custom_provider_auth(
                                    &provider_id,
                                    &base_url,
                                    &protocol,
                                    &api_key,
                                );
                            }
                        } else {
                            // Advance to next step
                            self.provider_dialog.advance_custom_flow();
                        }
                    }
                    KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.paste_clipboard_to_provider_dialog();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.provider_dialog.push_char(c);
                    }
                    _ => {}
                }
            } else if self.provider_dialog.is_input_mode() {
                // Known provider API key input mode
                match key.code {
                    KeyCode::Esc => self.provider_dialog.exit_input_mode(),
                    KeyCode::Backspace => {
                        self.provider_dialog.pop_char();
                    }
                    KeyCode::Enter => {
                        if let Some(PendingSubmit::Known {
                            provider_id,
                            api_key,
                        }) = self.provider_dialog.pending_submit()
                        {
                            self.submit_provider_auth(&provider_id, &api_key);
                        }
                    }
                    KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.paste_clipboard_to_provider_dialog();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.provider_dialog.push_char(c);
                    }
                    _ => {}
                }
            } else {
                // Provider list selection mode
                match key.code {
                    KeyCode::Esc => {
                        if self.provider_dialog.connect_mode == ProviderConnectMode::Known
                            && !self.provider_dialog.search_query.trim().is_empty()
                        {
                            self.provider_dialog.clear_search();
                        } else {
                            self.close_provider_dialog_modal();
                        }
                    }
                    KeyCode::Left => self.provider_dialog.toggle_mode_prev(),
                    KeyCode::Right => self.provider_dialog.toggle_mode_next(),
                    KeyCode::Tab => self.provider_dialog.toggle_mode_next(),
                    KeyCode::Up => self.provider_dialog.move_up(),
                    KeyCode::Down => self.provider_dialog.move_down(),
                    KeyCode::Enter => {
                        if self.provider_dialog.connect_mode == ProviderConnectMode::Custom {
                            self.provider_dialog.enter_input_mode();
                        } else if self.provider_dialog.connect_mode == ProviderConnectMode::Models {
                            self.provider_dialog.start_model_override_edit();
                        } else {
                            self.quick_connect_provider_dialog_selection();
                        }
                    }
                    KeyCode::Char('a')
                        if self.provider_dialog.connect_mode == ProviderConnectMode::Known =>
                    {
                        self.start_advanced_provider_dialog_selection();
                    }
                    KeyCode::Backspace
                        if self.provider_dialog.connect_mode == ProviderConnectMode::Known =>
                    {
                        self.provider_dialog.pop_search_char();
                        self.resolve_provider_dialog_search();
                    }
                    KeyCode::Char(c)
                        if self.provider_dialog.connect_mode == ProviderConnectMode::Known
                            && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        self.provider_dialog.push_search_char(c);
                        self.resolve_provider_dialog_search();
                    }
                    KeyCode::Char('n')
                        if self.provider_dialog.connect_mode == ProviderConnectMode::Models =>
                    {
                        self.provider_dialog.enter_input_mode();
                    }
                    KeyCode::Char('e')
                        if self.provider_dialog.connect_mode == ProviderConnectMode::Models =>
                    {
                        self.provider_dialog.start_model_override_edit();
                    }
                    KeyCode::Char('d')
                        if self.provider_dialog.connect_mode == ProviderConnectMode::Models =>
                    {
                        self.delete_selected_provider_model_override();
                    }
                    _ => {}
                }
            }
            return Ok(true);
        }

        if self.subagent_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_subagent_dialog_modal(),
                KeyCode::Up => self.subagent_dialog.scroll_up(),
                KeyCode::Down => self.subagent_dialog.scroll_down(50),
                _ => {}
            }
            return Ok(true);
        }

        if self.tag_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.close_tag_dialog_modal(),
                KeyCode::Up => self.tag_dialog.move_up(),
                KeyCode::Down => self.tag_dialog.move_down(),
                KeyCode::Char(' ') => self.tag_dialog.toggle_selection(),
                KeyCode::Enter => self.close_tag_dialog_modal(),
                _ => {}
            }
            return Ok(true);
        }

        if self.skill_proposal_review_dialog.is_open() {
            match key.code {
                KeyCode::Esc => {
                    self.close_skill_proposal_review_dialog();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.skill_proposal_review_dialog.previous();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.skill_proposal_review_dialog.next();
                }
                KeyCode::Char('a') => {
                    self.skill_proposal_review_dialog.pending_accept();
                    if let Some((_action, id)) = self.skill_proposal_review_dialog.take_action() {
                        self.apply_skill_proposal_action(&id, "accepted");
                    }
                }
                KeyCode::Char('r') => {
                    self.skill_proposal_review_dialog.pending_reject();
                    if let Some((_action, id)) = self.skill_proposal_review_dialog.take_action() {
                        self.apply_skill_proposal_action(&id, "rejected");
                    }
                }
                KeyCode::Char('s') => {
                    self.skill_proposal_review_dialog.pending_skip();
                    self.skill_proposal_review_dialog.remove_current();
                }
                _ => {}
            }
            return Ok(true);
        }

        Ok(false)
    }

    pub(super) fn handle_status_dialog_mouse(
        &mut self,
        button: crossterm::event::MouseButton,
        col: u16,
        row: u16,
    ) -> bool {
        if !self.status_dialog.is_open() {
            return false;
        }

        if self.status_dialog.handle_click(col, row) {
            self.close_status_dialog_modal();
            return true;
        }
        if self.status_dialog.contains_point(col, row) {
            if button == crossterm::event::MouseButton::Left {
                if let Some(area) = self.status_dialog.selection_area() {
                    if col >= area.x
                        && col < area.x.saturating_add(area.width)
                        && row >= area.y
                        && row < area.y.saturating_add(area.height)
                    {
                        self.selection.start_scoped(row, col, Some(area));
                    } else {
                        self.selection.clear();
                    }
                }
            }
            return true;
        }

        false
    }
}

fn change_summary(change: &agendao_types::SuggestedSkillChange) -> String {
    match change {
        agendao_types::SuggestedSkillChange::AddTriggerCondition { text, .. } => {
            format!("Trigger: {}", truncate_str(text, 60))
        }
        agendao_types::SuggestedSkillChange::AddCoreStep { text, .. } => {
            format!("Step: {}", truncate_str(text, 60))
        }
        agendao_types::SuggestedSkillChange::AddBoundary { text, .. } => {
            format!("Boundary: {}", truncate_str(text, 60))
        }
        agendao_types::SuggestedSkillChange::AddValidationStep { text, .. } => {
            format!("Validation: {}", truncate_str(text, 60))
        }
        agendao_types::SuggestedSkillChange::CreateSkillDraft { suggested_name, .. } => {
            format!("Create skill: {}", suggested_name)
        }
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
