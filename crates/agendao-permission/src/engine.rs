use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use agendao_plugin::{HookContext, HookEvent};

use crate::matching::wildcard_match;
use crate::{
    evaluate_permission_patterns, tool_to_permission, PermissionAction, PermissionRuleset,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionClass {
    InspectRead,
    WorkspaceWrite,
    ExternalAccess,
    DangerousExec,
}

impl PermissionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InspectRead => "inspect_read",
            Self::WorkspaceWrite => "workspace_write",
            Self::ExternalAccess => "external_access",
            Self::DangerousExec => "dangerous_exec",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLifetime {
    Once,
    Turn,
    Session,
}

impl PermissionLifetime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Turn => "turn",
            Self::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMatcherKind {
    ScopeOnly,
    ExactInput,
    StructuredFamily,
    SemanticPattern,
}

impl PermissionMatcherKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScopeOnly => "scope_only",
            Self::ExactInput => "exact_input",
            Self::StructuredFamily => "structured_family",
            Self::SemanticPattern => "semantic_pattern",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionGrantDescriptor {
    pub permission_class: Option<PermissionClass>,
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub matcher_kind: Option<PermissionMatcherKind>,
    #[serde(default)]
    pub matcher_key: Option<String>,
    #[serde(default)]
    pub origin_tool: Option<String>,
    #[serde(default)]
    pub risk_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionInfo {
    pub id: String,
    pub permission_type: String,
    pub pattern: Option<Pattern>,
    #[serde(default)]
    pub permission_class: Option<PermissionClass>,
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub matcher_kind: Option<PermissionMatcherKind>,
    #[serde(default)]
    pub matcher_key: Option<String>,
    #[serde(default)]
    pub origin_tool: Option<String>,
    #[serde(default)]
    pub risk_tags: Vec<String>,
    #[serde(default)]
    pub supported_lifetimes: Vec<PermissionLifetime>,
    pub session_id: String,
    pub message_id: String,
    pub call_id: Option<String>,
    pub message: String,
    pub metadata: HashMap<String, serde_json::Value>,
    pub time: TimeInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeInfo {
    pub created: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Pattern {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Once,
    Turn,
    Always,
    Reject,
}

#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub info: PermissionInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskOutcome {
    Granted,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct PermissionGrantKey {
    permission_class: Option<PermissionClass>,
    #[serde(default)]
    scope_key: Option<String>,
    #[serde(default)]
    matcher_kind: Option<PermissionMatcherKind>,
    #[serde(default)]
    matcher_key: Option<String>,
}

pub struct PermissionEngine {
    pending: HashMap<String, HashMap<String, PendingPermission>>,
    approved: HashMap<String, HashMap<String, bool>>,
    turn_approved: HashMap<String, HashMap<String, bool>>,
}

impl PermissionEngine {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            approved: HashMap::new(),
            turn_approved: HashMap::new(),
        }
    }

    pub fn pending(&self) -> &HashMap<String, HashMap<String, PendingPermission>> {
        &self.pending
    }

    pub fn list(&self) -> Vec<&PermissionInfo> {
        let mut result: Vec<&PermissionInfo> = Vec::new();
        for items in self.pending.values() {
            for item in items.values() {
                result.push(&item.info);
            }
        }
        result.sort_by(|a, b| a.id.cmp(&b.id));
        result
    }

    pub fn find(&self, permission_id: &str) -> Option<&PermissionInfo> {
        self.pending
            .values()
            .find_map(|items| items.get(permission_id).map(|item| &item.info))
    }

    fn scope_namespace(permission_class: Option<PermissionClass>, permission_type: &str) -> String {
        permission_class
            .map(|class| class.as_str().to_string())
            .unwrap_or_else(|| permission_type.to_string())
    }

    fn descriptor_from_parts(
        permission_class: Option<PermissionClass>,
        scope_key: Option<&str>,
        matcher_kind: Option<PermissionMatcherKind>,
        matcher_key: Option<&str>,
        origin_tool: Option<&str>,
    ) -> PermissionGrantDescriptor {
        PermissionGrantDescriptor {
            permission_class,
            scope_key: scope_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            matcher_kind,
            matcher_key: matcher_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            origin_tool: origin_tool
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            risk_tags: Vec::new(),
        }
    }

    fn serialize_key(key: &PermissionGrantKey) -> String {
        serde_json::to_string(key).unwrap_or_else(|_| {
            format!(
                "{}|{}|{}|{}",
                key.permission_class
                    .map(PermissionClass::as_str)
                    .unwrap_or("unknown"),
                key.scope_key.as_deref().unwrap_or("*"),
                key.matcher_kind
                    .map(PermissionMatcherKind::as_str)
                    .unwrap_or("legacy"),
                key.matcher_key.as_deref().unwrap_or("*")
            )
        })
    }

    fn legacy_to_keys(
        permission_class: Option<PermissionClass>,
        scope_key: Option<&str>,
        pattern: Option<&Pattern>,
        permission_type: &str,
    ) -> Vec<String> {
        if let Some(scope_key) = scope_key.filter(|value| !value.trim().is_empty()) {
            return vec![format!(
                "{}|{}",
                Self::scope_namespace(permission_class, permission_type),
                scope_key
            )];
        }

        match pattern {
            None => vec![permission_type.to_string()],
            Some(Pattern::Single(s)) => vec![s.clone()],
            Some(Pattern::Multiple(v)) => v.clone(),
        }
    }

    fn descriptor_to_keys(
        descriptor: &PermissionGrantDescriptor,
        pattern: Option<&Pattern>,
        permission_type: &str,
    ) -> Vec<String> {
        let mut keys = Vec::new();

        if let Some(scope_key) = descriptor
            .scope_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if matches!(
                descriptor.matcher_kind,
                None | Some(PermissionMatcherKind::ScopeOnly)
            ) {
                keys.push(Self::serialize_key(&PermissionGrantKey {
                    permission_class: descriptor.permission_class,
                    scope_key: Some(scope_key.to_string()),
                    matcher_kind: Some(PermissionMatcherKind::ScopeOnly),
                    matcher_key: Some(scope_key.to_string()),
                }));
            }
        }

        if let Some(matcher_kind) = descriptor.matcher_kind {
            match matcher_kind {
                PermissionMatcherKind::ScopeOnly => {}
                PermissionMatcherKind::ExactInput
                | PermissionMatcherKind::StructuredFamily
                | PermissionMatcherKind::SemanticPattern => {
                    if let Some(matcher_key) = descriptor
                        .matcher_key
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        keys.push(Self::serialize_key(&PermissionGrantKey {
                            permission_class: descriptor.permission_class,
                            scope_key: descriptor.scope_key.clone(),
                            matcher_kind: Some(matcher_kind),
                            matcher_key: Some(matcher_key.to_string()),
                        }));
                    }
                }
            }
        }

        if keys.is_empty() {
            keys.extend(Self::legacy_to_keys(
                descriptor.permission_class,
                descriptor.scope_key.as_deref(),
                pattern,
                permission_type,
            ));
            return keys;
        }

        keys.extend(Self::legacy_to_keys(
            descriptor.permission_class,
            descriptor.scope_key.as_deref(),
            pattern,
            permission_type,
        ));
        keys.sort();
        keys.dedup();
        keys
    }

    fn to_keys(
        permission_class: Option<PermissionClass>,
        scope_key: Option<&str>,
        matcher_kind: Option<PermissionMatcherKind>,
        matcher_key: Option<&str>,
        origin_tool: Option<&str>,
        pattern: Option<&Pattern>,
        permission_type: &str,
    ) -> Vec<String> {
        let descriptor = Self::descriptor_from_parts(
            permission_class,
            scope_key,
            matcher_kind,
            matcher_key,
            origin_tool,
        );
        Self::descriptor_to_keys(&descriptor, pattern, permission_type)
    }

    fn patterns(pattern: Option<&Pattern>) -> Vec<String> {
        match pattern {
            None => Vec::new(),
            Some(Pattern::Single(s)) => vec![s.clone()],
            Some(Pattern::Multiple(v)) => v.clone(),
        }
    }

    fn covered(keys: &[String], approved: &HashMap<String, bool>) -> bool {
        let patterns: Vec<&String> = approved.keys().collect();
        keys.iter()
            .all(|k| patterns.iter().any(|p| wildcard_match(k, p)))
    }

    pub fn is_approved(
        &self,
        session_id: &str,
        permission_class: Option<PermissionClass>,
        scope_key: Option<&str>,
        matcher_kind: Option<PermissionMatcherKind>,
        matcher_key: Option<&str>,
        origin_tool: Option<&str>,
        pattern: Option<&Pattern>,
        permission_type: &str,
    ) -> bool {
        let empty = HashMap::new();
        let approved_for_session = self.approved.get(session_id).unwrap_or(&empty);
        let keys = Self::to_keys(
            permission_class,
            scope_key,
            matcher_kind,
            matcher_key,
            origin_tool,
            pattern,
            permission_type,
        );
        Self::covered(&keys, approved_for_session)
    }

    pub fn is_turn_approved(
        &self,
        session_id: &str,
        permission_class: Option<PermissionClass>,
        scope_key: Option<&str>,
        matcher_kind: Option<PermissionMatcherKind>,
        matcher_key: Option<&str>,
        origin_tool: Option<&str>,
        pattern: Option<&Pattern>,
        permission_type: &str,
    ) -> bool {
        let empty = HashMap::new();
        let approved_for_turn = self.turn_approved.get(session_id).unwrap_or(&empty);
        let keys = Self::to_keys(
            permission_class,
            scope_key,
            matcher_kind,
            matcher_key,
            origin_tool,
            pattern,
            permission_type,
        );
        Self::covered(&keys, approved_for_turn)
    }

    pub fn grant(
        &mut self,
        session_id: &str,
        permission_class: Option<PermissionClass>,
        scope_key: Option<&str>,
        matcher_kind: Option<PermissionMatcherKind>,
        matcher_key: Option<&str>,
        origin_tool: Option<&str>,
        permission_type: &str,
        pattern: Option<&Pattern>,
    ) {
        let approved_session = self.approved.entry(session_id.to_string()).or_default();
        for key in Self::to_keys(
            permission_class,
            scope_key,
            matcher_kind,
            matcher_key,
            origin_tool,
            pattern,
            permission_type,
        ) {
            approved_session.insert(key, true);
        }
    }

    pub fn grant_turn(
        &mut self,
        session_id: &str,
        permission_class: Option<PermissionClass>,
        scope_key: Option<&str>,
        matcher_kind: Option<PermissionMatcherKind>,
        matcher_key: Option<&str>,
        origin_tool: Option<&str>,
        permission_type: &str,
        pattern: Option<&Pattern>,
    ) {
        let approved_turn = self
            .turn_approved
            .entry(session_id.to_string())
            .or_default();
        for key in Self::to_keys(
            permission_class,
            scope_key,
            matcher_kind,
            matcher_key,
            origin_tool,
            pattern,
            permission_type,
        ) {
            approved_turn.insert(key, true);
        }
    }

    pub fn grant_patterns(&mut self, session_id: &str, permission_type: &str, patterns: &[String]) {
        let pattern = match patterns {
            [] => None,
            [single] => Some(Pattern::Single(single.clone())),
            _ => Some(Pattern::Multiple(patterns.to_vec())),
        };
        self.grant(
            session_id,
            None,
            None,
            None,
            None,
            None,
            permission_type,
            pattern.as_ref(),
        );
    }

    pub fn evaluate_tool(
        tool_name: &str,
        allowed_tools: &[String],
        rulesets: &[PermissionRuleset],
    ) -> PermissionAction {
        Self::evaluate_tool_with_patterns(tool_name, &[], allowed_tools, rulesets)
    }

    pub fn evaluate_tool_with_patterns(
        tool_name: &str,
        patterns: &[String],
        allowed_tools: &[String],
        rulesets: &[PermissionRuleset],
    ) -> PermissionAction {
        if !allowed_tools.is_empty() && !allowed_tools.iter().any(|tool| tool == tool_name) {
            return PermissionAction::Deny;
        }

        let permission = tool_to_permission(tool_name);
        evaluate_permission_patterns(permission, patterns, rulesets)
    }

    pub async fn ask(&mut self, info: PermissionInfo) -> Result<AskOutcome, PermissionError> {
        self.ask_with_rules(info, &[]).await
    }

    pub async fn ask_with_rules(
        &mut self,
        info: PermissionInfo,
        rulesets: &[PermissionRuleset],
    ) -> Result<AskOutcome, PermissionError> {
        let session_id = info.session_id.clone();
        let permission_id = info.id.clone();
        let patterns = Self::patterns(info.pattern.as_ref());

        if matches!(info.permission_class, Some(PermissionClass::InspectRead)) {
            return Ok(AskOutcome::Granted);
        }

        if self.is_approved(
            &session_id,
            info.permission_class,
            info.scope_key.as_deref(),
            info.matcher_kind,
            info.matcher_key.as_deref(),
            info.origin_tool.as_deref(),
            info.pattern.as_ref(),
            &info.permission_type,
        ) {
            return Ok(AskOutcome::Granted);
        }

        if self.is_turn_approved(
            &session_id,
            info.permission_class,
            info.scope_key.as_deref(),
            info.matcher_kind,
            info.matcher_key.as_deref(),
            info.origin_tool.as_deref(),
            info.pattern.as_ref(),
            &info.permission_type,
        ) {
            return Ok(AskOutcome::Granted);
        }

        match evaluate_permission_patterns(&info.permission_type, &patterns, rulesets) {
            PermissionAction::Allow => return Ok(AskOutcome::Granted),
            PermissionAction::Deny => {
                return Err(PermissionError::Rejected {
                    session_id: session_id.clone(),
                    permission_id: permission_id.clone(),
                    tool_call_id: info.call_id.clone(),
                });
            }
            PermissionAction::Ask => {}
        }

        // Plugin hook: permission.ask — plugins may decide "ask" | "deny" | "allow".
        let mut hook_ctx = HookContext::new(HookEvent::PermissionAsk)
            .with_session(&session_id)
            .with_data("permission_type", serde_json::json!(&info.permission_type))
            .with_data("permission_id", serde_json::json!(&permission_id))
            .with_data("permission", serde_json::json!(&info))
            .with_data("status", serde_json::json!("ask"));
        if let Some(call_id) = &info.call_id {
            hook_ctx = hook_ctx.with_data("call_id", serde_json::json!(call_id));
        }

        let mut status = "ask".to_string();
        let hook_outputs = agendao_plugin::trigger_collect(hook_ctx).await;
        for output in hook_outputs {
            let Some(payload) = output.payload.as_ref() else {
                continue;
            };
            if let Some(next_status) = extract_permission_status(payload) {
                status = next_status;
            }
        }

        match status.as_str() {
            "allow" => return Ok(AskOutcome::Granted),
            "deny" => {
                return Err(PermissionError::Rejected {
                    session_id: session_id.clone(),
                    permission_id: permission_id.clone(),
                    tool_call_id: info.call_id.clone(),
                });
            }
            _ => {}
        }

        self.pending
            .entry(session_id.clone())
            .or_default()
            .insert(permission_id, PendingPermission { info });

        Ok(AskOutcome::Pending)
    }

    pub fn respond(
        &mut self,
        session_id: &str,
        permission_id: &str,
        response: Response,
    ) -> Result<(), PermissionError> {
        let session_pending = self.pending.get_mut(session_id).ok_or_else(|| {
            PermissionError::NotFound(session_id.to_string(), permission_id.to_string())
        })?;

        let match_item = session_pending.remove(permission_id).ok_or_else(|| {
            PermissionError::NotFound(session_id.to_string(), permission_id.to_string())
        })?;

        if response == Response::Reject {
            return Err(PermissionError::Rejected {
                session_id: session_id.to_string(),
                permission_id: permission_id.to_string(),
                tool_call_id: match_item.info.call_id.clone(),
            });
        }

        match response {
            Response::Always => {
                self.grant(
                    session_id,
                    match_item.info.permission_class,
                    match_item.info.scope_key.as_deref(),
                    match_item.info.matcher_kind,
                    match_item.info.matcher_key.as_deref(),
                    match_item.info.origin_tool.as_deref(),
                    &match_item.info.permission_type,
                    match_item.info.pattern.as_ref(),
                );
            }
            Response::Turn => {
                self.grant_turn(
                    session_id,
                    match_item.info.permission_class,
                    match_item.info.scope_key.as_deref(),
                    match_item.info.matcher_kind,
                    match_item.info.matcher_key.as_deref(),
                    match_item.info.origin_tool.as_deref(),
                    &match_item.info.permission_type,
                    match_item.info.pattern.as_ref(),
                );
            }
            Response::Once | Response::Reject => {}
        }

        Ok(())
    }

    pub fn respond_by_id(
        &mut self,
        permission_id: &str,
        response: Response,
    ) -> Result<PermissionInfo, PermissionError> {
        let session_id = self
            .pending
            .iter()
            .find_map(|(session_id, items)| items.contains_key(permission_id).then_some(session_id))
            .cloned()
            .ok_or_else(|| PermissionError::NotFound("*".to_string(), permission_id.to_string()))?;

        let info = self.find(permission_id).cloned().ok_or_else(|| {
            PermissionError::NotFound(session_id.clone(), permission_id.to_string())
        })?;

        self.respond(&session_id, permission_id, response)?;
        Ok(info)
    }

    pub fn remove_pending(&mut self, permission_id: &str) -> Option<PermissionInfo> {
        let session_id = self
            .pending
            .iter()
            .find_map(|(session_id, items)| items.contains_key(permission_id).then_some(session_id))
            .cloned()?;

        let items = self.pending.get_mut(&session_id)?;
        let removed = items.remove(permission_id)?;
        if items.is_empty() {
            self.pending.remove(&session_id);
        }
        Some(removed.info)
    }

    pub fn clear_session(&mut self, session_id: &str) {
        self.pending.remove(session_id);
        self.approved.remove(session_id);
        self.turn_approved.remove(session_id);
    }

    pub fn clear_turn(&mut self, session_id: &str) {
        self.turn_approved.remove(session_id);
    }
}

impl Default for PermissionEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_permission_status(payload: &serde_json::Value) -> Option<String> {
    agendao_plugin::hook_payload_object(payload)
        .and_then(|object| object.get("status"))
        .and_then(|value| value.as_str())
        .filter(|status| matches!(*status, "ask" | "deny" | "allow"))
        .map(ToString::to_string)
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("Permission not found: {0}/{1}")]
    NotFound(String, String),

    #[error("Permission rejected")]
    Rejected {
        session_id: String,
        permission_id: String,
        tool_call_id: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_permission_engine() {
        let mut engine = PermissionEngine::new();

        let info = PermissionInfo {
            id: "per_test".to_string(),
            permission_type: "bash".to_string(),
            pattern: Some(Pattern::Single("ls".to_string())),
            permission_class: Some(PermissionClass::DangerousExec),
            scope_key: Some("cmd:ls".to_string()),
            matcher_kind: Some(PermissionMatcherKind::StructuredFamily),
            matcher_key: Some("cmd:ls".to_string()),
            origin_tool: Some("bash".to_string()),
            risk_tags: vec!["dangerous_exec".to_string()],
            supported_lifetimes: vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ],
            session_id: "ses_test".to_string(),
            message_id: "msg_test".to_string(),
            call_id: None,
            message: "Execute ls command".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        engine.ask(info).await.unwrap();
        assert!(!engine.list().is_empty());

        engine
            .respond("ses_test", "per_test", Response::Once)
            .unwrap();
        assert!(engine.list().is_empty());
    }

    #[tokio::test]
    async fn turn_grant_auto_approves_same_permission_until_cleared() {
        let mut engine = PermissionEngine::new();

        let info = PermissionInfo {
            id: "per_turn".to_string(),
            permission_type: "bash".to_string(),
            pattern: Some(Pattern::Single("cargo test".to_string())),
            permission_class: Some(PermissionClass::DangerousExec),
            scope_key: Some("cmd:cargo *".to_string()),
            matcher_kind: Some(PermissionMatcherKind::StructuredFamily),
            matcher_key: Some("cmd:cargo *".to_string()),
            origin_tool: Some("bash".to_string()),
            risk_tags: vec!["dangerous_exec".to_string()],
            supported_lifetimes: vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ],
            session_id: "ses_turn".to_string(),
            message_id: "msg_turn".to_string(),
            call_id: None,
            message: "Execute cargo test".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        engine.ask(info).await.unwrap();
        engine
            .respond("ses_turn", "per_turn", Response::Turn)
            .unwrap();

        assert!(engine.is_turn_approved(
            "ses_turn",
            Some(PermissionClass::DangerousExec),
            Some("cmd:cargo *"),
            Some(PermissionMatcherKind::StructuredFamily),
            Some("cmd:cargo *"),
            Some("bash"),
            Some(&Pattern::Single("cargo test".to_string())),
            "bash"
        ));

        engine.clear_turn("ses_turn");

        assert!(!engine.is_turn_approved(
            "ses_turn",
            Some(PermissionClass::DangerousExec),
            Some("cmd:cargo *"),
            Some(PermissionMatcherKind::StructuredFamily),
            Some("cmd:cargo *"),
            Some("bash"),
            Some(&Pattern::Single("cargo test".to_string())),
            "bash"
        ));
    }

    #[tokio::test]
    async fn scope_key_grant_applies_across_different_patterns_in_same_scope() {
        let mut engine = PermissionEngine::new();

        let first = PermissionInfo {
            id: "per_scope_a".to_string(),
            permission_type: "edit".to_string(),
            pattern: Some(Pattern::Single("/repo/src/a.rs".to_string())),
            permission_class: Some(PermissionClass::WorkspaceWrite),
            scope_key: Some("workspace:/src".to_string()),
            matcher_kind: Some(PermissionMatcherKind::ScopeOnly),
            matcher_key: Some("workspace:/src".to_string()),
            origin_tool: Some("edit".to_string()),
            risk_tags: vec!["workspace_write".to_string()],
            supported_lifetimes: vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ],
            session_id: "ses_scope".to_string(),
            message_id: "msg_scope_a".to_string(),
            call_id: None,
            message: "Edit src/a.rs".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        engine.ask(first).await.unwrap();
        engine
            .respond("ses_scope", "per_scope_a", Response::Always)
            .unwrap();

        let second = PermissionInfo {
            id: "per_scope_b".to_string(),
            permission_type: "edit".to_string(),
            pattern: Some(Pattern::Single("/repo/src/b.rs".to_string())),
            permission_class: Some(PermissionClass::WorkspaceWrite),
            scope_key: Some("workspace:/src".to_string()),
            matcher_kind: Some(PermissionMatcherKind::ScopeOnly),
            matcher_key: Some("workspace:/src".to_string()),
            origin_tool: Some("edit".to_string()),
            risk_tags: vec!["workspace_write".to_string()],
            supported_lifetimes: vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ],
            session_id: "ses_scope".to_string(),
            message_id: "msg_scope_b".to_string(),
            call_id: None,
            message: "Edit src/b.rs".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        assert!(matches!(engine.ask(second).await, Ok(AskOutcome::Granted)));
    }

    #[tokio::test]
    async fn structured_family_session_grant_matches_same_family_only() {
        let mut engine = PermissionEngine::new();

        let first = PermissionInfo {
            id: "per_family_a".to_string(),
            permission_type: "bash".to_string(),
            pattern: Some(Pattern::Single("cargo check".to_string())),
            permission_class: Some(PermissionClass::DangerousExec),
            scope_key: Some("cmd:cargo *".to_string()),
            matcher_kind: Some(PermissionMatcherKind::StructuredFamily),
            matcher_key: Some("cmd:cargo *".to_string()),
            origin_tool: Some("bash".to_string()),
            risk_tags: vec!["dangerous_exec".to_string()],
            supported_lifetimes: vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ],
            session_id: "ses_family".to_string(),
            message_id: "msg_family_a".to_string(),
            call_id: None,
            message: "Execute cargo check".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        engine.ask(first).await.unwrap();
        engine
            .respond("ses_family", "per_family_a", Response::Always)
            .unwrap();

        let same_family = PermissionInfo {
            id: "per_family_b".to_string(),
            permission_type: "bash".to_string(),
            pattern: Some(Pattern::Single("cargo test".to_string())),
            permission_class: Some(PermissionClass::DangerousExec),
            scope_key: Some("cmd:cargo *".to_string()),
            matcher_kind: Some(PermissionMatcherKind::StructuredFamily),
            matcher_key: Some("cmd:cargo *".to_string()),
            origin_tool: Some("bash".to_string()),
            risk_tags: vec!["dangerous_exec".to_string()],
            supported_lifetimes: vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ],
            session_id: "ses_family".to_string(),
            message_id: "msg_family_b".to_string(),
            call_id: None,
            message: "Execute cargo test".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        assert!(matches!(
            engine.ask(same_family).await,
            Ok(AskOutcome::Granted)
        ));

        let different_family = PermissionInfo {
            id: "per_family_c".to_string(),
            permission_type: "bash".to_string(),
            pattern: Some(Pattern::Single("git status".to_string())),
            permission_class: Some(PermissionClass::DangerousExec),
            scope_key: Some("cmd:git *".to_string()),
            matcher_kind: Some(PermissionMatcherKind::StructuredFamily),
            matcher_key: Some("cmd:git *".to_string()),
            origin_tool: Some("bash".to_string()),
            risk_tags: vec!["dangerous_exec".to_string()],
            supported_lifetimes: vec![
                PermissionLifetime::Once,
                PermissionLifetime::Turn,
                PermissionLifetime::Session,
            ],
            session_id: "ses_family".to_string(),
            message_id: "msg_family_c".to_string(),
            call_id: None,
            message: "Execute git status".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        assert!(matches!(
            engine.ask(different_family).await,
            Ok(AskOutcome::Pending)
        ));
    }

    #[tokio::test]
    async fn exact_input_grant_does_not_cover_different_input() {
        let mut engine = PermissionEngine::new();

        let first = PermissionInfo {
            id: "per_exact_a".to_string(),
            permission_type: "bash".to_string(),
            pattern: Some(Pattern::Single("rm -rf /tmp/x".to_string())),
            permission_class: Some(PermissionClass::DangerousExec),
            scope_key: None,
            matcher_kind: Some(PermissionMatcherKind::ExactInput),
            matcher_key: Some("rm -rf /tmp/x".to_string()),
            origin_tool: Some("bash".to_string()),
            risk_tags: vec!["dangerous_exec".to_string()],
            supported_lifetimes: vec![PermissionLifetime::Once],
            session_id: "ses_exact".to_string(),
            message_id: "msg_exact_a".to_string(),
            call_id: None,
            message: "Execute exact command".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        engine.ask(first).await.unwrap();
        engine
            .respond("ses_exact", "per_exact_a", Response::Always)
            .unwrap();

        let second = PermissionInfo {
            id: "per_exact_b".to_string(),
            permission_type: "bash".to_string(),
            pattern: Some(Pattern::Single("rm -rf /tmp/y".to_string())),
            permission_class: Some(PermissionClass::DangerousExec),
            scope_key: None,
            matcher_kind: Some(PermissionMatcherKind::ExactInput),
            matcher_key: Some("rm -rf /tmp/y".to_string()),
            origin_tool: Some("bash".to_string()),
            risk_tags: vec!["dangerous_exec".to_string()],
            supported_lifetimes: vec![PermissionLifetime::Once],
            session_id: "ses_exact".to_string(),
            message_id: "msg_exact_b".to_string(),
            call_id: None,
            message: "Execute different command".to_string(),
            metadata: HashMap::new(),
            time: TimeInfo { created: 0 },
        };

        assert!(matches!(engine.ask(second).await, Ok(AskOutcome::Pending)));
    }

    #[test]
    fn test_wildcard_match() {
        assert!(wildcard_match("foo", "*"));
        assert!(wildcard_match("foo/bar", "foo/*"));
        assert!(wildcard_match("foo/bar/baz", "*/baz"));
        assert!(wildcard_match("foo/bar/baz", "*bar*"));
        assert!(!wildcard_match("foo", "bar"));
    }
}
