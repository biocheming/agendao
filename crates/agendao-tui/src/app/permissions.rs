use std::collections::HashSet;

use chrono::{SecondsFormat, Utc};

use crossterm::event::{KeyCode, KeyEvent};

use crate::components::{PermissionAction, PermissionLifetime, PermissionRequest, PermissionType};
use crate::event::{CustomEvent, PermissionReplyOutcome};

use super::App;

impl App {
    fn default_supported_lifetimes(permission_class: Option<&str>) -> Vec<PermissionLifetime> {
        match permission_class {
            Some("workspace_write" | "external_access") => vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ],
            Some("inspect_read" | "dangerous_exec") => vec![PermissionLifetime::Once],
            Some(_) | None => vec![PermissionLifetime::Once],
        }
    }

    fn permission_type_from_name(name: &str) -> PermissionType {
        match name {
            "read" => PermissionType::ReadFile,
            "write" => PermissionType::WriteFile,
            "edit" => PermissionType::Edit,
            "bash" => PermissionType::Bash,
            "glob" => PermissionType::Glob,
            "grep" => PermissionType::Grep,
            "list" => PermissionType::List,
            "task" | "task_flow" => PermissionType::Task,
            "webfetch" => PermissionType::WebFetch,
            "websearch" => PermissionType::WebSearch,
            "codesearch" => PermissionType::CodeSearch,
            "external_directory" => PermissionType::ExternalDirectory,
            _ => PermissionType::ExecuteCommand,
        }
    }

    fn permission_request_to_prompt(
        permission: &crate::api::PermissionRequestInfo,
    ) -> PermissionRequest {
        let input = permission.input.as_object().cloned().unwrap_or_default();
        let permission_name = input
            .get("permission")
            .and_then(|value| value.as_str())
            .unwrap_or(permission.tool.as_str());
        let resource = input
            .get("patterns")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|value| !value.is_empty())
            .or_else(|| {
                input.get("metadata").and_then(|value| {
                    value
                        .get("command")
                        .and_then(|item| item.as_str())
                        .or_else(|| value.get("filepath").and_then(|item| item.as_str()))
                        .or_else(|| value.get("path").and_then(|item| item.as_str()))
                        .map(str::to_string)
                })
            })
            .unwrap_or_else(|| permission.message.clone());
        let supported_lifetimes = if !permission.supported_lifetimes.is_empty() {
            Some(
                permission
                    .supported_lifetimes
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )
        } else {
            input
                .get("supported_lifetimes")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                })
        }
        .map(|values| {
            values
                .iter()
                .filter_map(|value| match *value {
                    "once" => Some(PermissionLifetime::Once),
                    "turn" => Some(PermissionLifetime::Turn),
                    "session" | "always" => Some(PermissionLifetime::Session),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| {
            Self::default_supported_lifetimes(permission.permission_class.as_deref())
        });

        PermissionRequest {
            id: permission.id.clone(),
            permission_type: Self::permission_type_from_name(permission_name),
            resource,
            tool_name: permission_name.to_string(),
            permission_class: permission.permission_class.clone(),
            scope_key: permission.scope_key.clone(),
            scope_label: permission.scope_label.clone(),
            matcher_label: permission.matcher_label.clone(),
            grant_target_summary: permission.grant_target_summary.clone(),
            risk_tags: permission.risk_tags.clone(),
            supported_lifetimes,
            is_submitting: false,
            submit_error: None,
        }
    }

    pub(super) fn sync_permission_requests(&mut self) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.context.set_pending_permissions(0);
            return false;
        };

        let active_session = self.current_session_id();
        let mut permissions = match client.list_permissions() {
            Ok(items) => items,
            Err(error) => {
                tracing::debug!(%error, "failed to sync permission requests");
                return false;
            }
        };

        if let Some(session_id) = active_session.as_deref() {
            permissions.retain(|permission| permission.session_id == session_id);
        }
        permissions.sort_by(|a, b| a.id.cmp(&b.id));

        let latest_ids = permissions
            .iter()
            .map(|permission| permission.id.clone())
            .collect::<HashSet<_>>();
        let mut changed = latest_ids != self.permission_runtime.pending_ids;

        for permission in permissions {
            let permission_id = permission.id.clone();
            if self
                .permission_runtime
                .pending_ids
                .insert(permission_id.clone())
            {
                self.permission_prompt
                    .add_request(Self::permission_request_to_prompt(&permission));
                changed = true;
            }
            self.permission_runtime
                .pending_requests
                .insert(permission_id, permission);
        }

        self.permission_runtime
            .pending_ids
            .retain(|id| latest_ids.contains(id));
        self.permission_runtime
            .pending_requests
            .retain(|id, _| latest_ids.contains(id));
        self.permission_prompt
            .retain_requests(|request| latest_ids.contains(&request.id));

        changed
    }

    pub(super) fn resolve_permission_request(
        &mut self,
        permission_id: &str,
        reply: &str,
        message: Option<String>,
    ) {
        let Some(client) = self.context.get_api_client() else {
            self.alert_dialog
                .set_message("Cannot answer permission request: no API client");
            self.open_alert_dialog();
            return;
        };

        if !self.permission_prompt.mark_submitting(permission_id) {
            return;
        }
        self.permission_runtime.last_submit_error = None;
        self.permission_runtime.last_submit_started_at =
            Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));

        let permission_id = permission_id.to_string();
        let reply = reply.to_string();
        let context = self.context.clone();
        std::thread::spawn(move || {
            let outcome = match client.reply_permission_priority(&permission_id, &reply, message) {
                Ok(()) => PermissionReplyOutcome::Succeeded,
                Err(error) => PermissionReplyOutcome::Failed {
                    message: error.to_string(),
                },
            };
            let _ = context.emit_custom_event(CustomEvent::PermissionReplyFinished {
                permission_id,
                outcome,
            });
        });
    }

    pub(super) fn enqueue_permission_request(
        &mut self,
        permission: crate::api::PermissionRequestInfo,
    ) {
        if self
            .permission_runtime
            .pending_ids
            .insert(permission.id.clone())
        {
            self.permission_prompt
                .add_request(Self::permission_request_to_prompt(&permission));
        }
        self.permission_runtime
            .pending_requests
            .insert(permission.id.clone(), permission);
        self.permission_runtime.last_submit_error = None;
    }

    pub(super) fn clear_permission_request(&mut self, permission_id: &str) {
        self.permission_runtime.pending_ids.remove(permission_id);
        self.permission_runtime
            .pending_requests
            .remove(permission_id);
        self.permission_prompt.clear_submit_state(permission_id);
        self.permission_prompt.remove_request(permission_id);
    }

    pub(super) fn resolve_permission_prompt_action(&mut self, action: PermissionAction) {
        match action {
            PermissionAction::ApproveOnce => {
                if let Some(request) = self.permission_prompt.approve_once() {
                    self.resolve_permission_request(
                        &request.id,
                        "once",
                        Some("approved once".to_string()),
                    );
                }
            }
            PermissionAction::ApproveTurn => {
                if let Some(request) = self.permission_prompt.approve_turn() {
                    self.resolve_permission_request(
                        &request.id,
                        "turn",
                        Some("approved for turn".to_string()),
                    );
                }
            }
            PermissionAction::ApproveSession => {
                if let Some(request) = self.permission_prompt.approve_session() {
                    self.resolve_permission_request(
                        &request.id,
                        "session",
                        Some("approved for session".to_string()),
                    );
                }
            }
            PermissionAction::Deny => {
                if let Some(request) = self.permission_prompt.deny() {
                    self.resolve_permission_request(
                        &request.id,
                        "reject",
                        Some("rejected".to_string()),
                    );
                }
            }
        }
    }

    pub(super) fn handle_permission_prompt_key(&mut self, key: KeyEvent) -> bool {
        if !self.permission_prompt.is_open {
            return false;
        }
        if self.permission_prompt.is_current_request_submitting() {
            return true;
        }

        match key.code {
            KeyCode::Char('y') | KeyCode::Enter | KeyCode::Char('1') => {
                self.resolve_permission_prompt_action(PermissionAction::ApproveOnce);
            }
            KeyCode::Char('0') | KeyCode::Char('n') | KeyCode::Esc => {
                self.resolve_permission_prompt_action(PermissionAction::Deny);
            }
            KeyCode::Char('2') => {
                self.resolve_permission_prompt_action(PermissionAction::ApproveTurn);
            }
            KeyCode::Char('3') | KeyCode::Char('a') => {
                self.resolve_permission_prompt_action(PermissionAction::ApproveSession);
            }
            _ => {}
        }

        true
    }

    pub(super) fn handle_permission_prompt_mouse(&mut self, col: u16, row: u16) -> bool {
        if !self.permission_prompt.is_open {
            return false;
        }

        self.permission_prompt.handle_click(col, row);
        if let Some(action) = self.permission_prompt.take_pending_action() {
            self.resolve_permission_prompt_action(action);
        }
        true
    }
}
