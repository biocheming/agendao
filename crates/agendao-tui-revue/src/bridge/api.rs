//! Synchronous API bridge.
//!
//! Wraps `agendao_client::AsyncApiClient` using a background tokio runtime.
//! Mirrors the full API surface of old TUI's RuntimeApiClient.

use std::sync::Arc;
use agendao_client::{AsyncApiClient, PromptResponse, SessionInfo};
use agendao_server_local::LocalServerState;
use agendao_state::RecentModelEntry;

#[derive(Clone)]
pub struct ApiBridge {
    client: Arc<AsyncApiClient>,
    /// In-process local server state for local-direct mode.
    local: Option<Arc<LocalServerState>>,
    handle: tokio::runtime::Handle,
}

impl ApiBridge {
    /// Create an HTTP-based bridge (connects to external server).
    pub fn new(base_url: &str, handle: tokio::runtime::Handle) -> anyhow::Result<Self> {
        let client = Arc::new(AsyncApiClient::new(base_url.to_string()));
        Ok(Self { client, local: None, handle })
    }

    /// Create a local-direct bridge (in-process, no HTTP).
    pub fn new_local(local: Arc<LocalServerState>, handle: tokio::runtime::Handle) -> Self {
        // AsyncApiClient still created for methods without local_* counterpart;
        // they will error at runtime but will never be called in normal flow.
        let client = Arc::new(AsyncApiClient::new("http://127.0.0.1:0".into()));
        Self { client, local: Some(local), handle }
    }

    fn block_on<R>(&self, fut: impl std::future::Future<Output = R>) -> R {
        self.handle.block_on(fut)
    }


    // ── Sessions ──

    pub fn create_session(
        &self, profile: Option<String>, directory: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        if let Some(ref ls) = self.local {
            use agendao_client::CreateSessionRequest;
            let req = CreateSessionRequest {
                scheduler_profile: profile,
                directory,
                project_id: None,
                title: None,
            };
            let result = self.block_on(
                agendao_server_local::local_create_session(Arc::clone(ls), req)
            )?;
            return Ok(result);
        }
        self.block_on(self.client.create_session(profile, directory))
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
        self.list_sessions_in_directory(None)
    }

    /// List sessions filtered by exact directory match (canonical path).
    ///
    /// 木 → 土 边界：UI 把当前 cwd（store.working_dir，已 canonicalize）传进来，
    /// 命中 session_record.directory（同样在创建时 canonicalize）。
    ///
    /// Sorted descending by `time.updated` so the most-recently-touched
    /// session lands at the top — UI never has to re-sort.
    pub fn list_sessions_in_directory(
        &self,
        directory: Option<String>,
    ) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
        let mut items = if let Some(ref ls) = self.local {
            self.block_on(agendao_server_local::local_list_sessions_in_directory(
                Arc::clone(ls),
                directory.clone(),
                None,
                None,
            ))?
        } else {
            self.block_on(self.client.list_sessions_in_directory(
                directory.as_deref(),
                None,
                None,
            ))?
        };
        // Most recent first; ties keep insertion order via sort_by (stable).
        sort_sessions_recent_first(&mut items);
        Ok(items)
    }

    pub fn get_session(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_get_session(Arc::clone(ls), session_id));
        }
        self.block_on(self.client.get_session(session_id))
    }

    pub fn get_messages(&self, session_id: &str) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_list_messages(Arc::clone(ls), session_id, None, None));
        }
        self.block_on(self.client.get_messages(session_id))
    }

    pub fn send_prompt(
        &self, session_id: &str, content: String,
    ) -> anyhow::Result<PromptResponse> {
        self.send_prompt_with(session_id, content, None, None, None, None)
    }

    /// Send a prompt carrying explicit agent/model/variant/profile selections.
    ///
    /// `dispatch()` calls this with the user's current selections from the UI
    /// store. Without these, the server falls back to its default profile
    /// regardless of what `/models` or `/agents` chose — the bug surfaced as
    /// 401 errors against `zhipuai-coding-plan/glm-5.1` even after the user
    /// switched to a DeepSeek model in the dialog.
    pub fn send_prompt_with(
        &self,
        session_id: &str,
        content: String,
        agent: Option<String>,
        scheduler_profile: Option<String>,
        model: Option<String>,
        variant: Option<String>,
    ) -> anyhow::Result<PromptResponse> {
        if let Some(ref ls) = self.local {
            use agendao_client::PromptRequest;
            let req = PromptRequest {
                message: Some(content),
                parts: None,
                agent,
                scheduler_profile,
                model,
                variant,
                ingress_source: None,
                idempotency_key: None,
                source_origin: None,
                source_surface: None,
                command: None,
                arguments: None,
            };
            return self.block_on(agendao_server_local::local_prompt(Arc::clone(ls), session_id, req));
        }
        let c = Arc::clone(&self.client);
        self.block_on(c.send_prompt(
            session_id, content,
            None, agent, scheduler_profile, model, variant, None, None, None, None, None,
        ))
    }

    // ── Models & Providers ──

    pub fn get_all_providers(&self) -> anyhow::Result<agendao_client::FullProviderListResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_get_all_providers(Arc::clone(ls)));
        }
        self.block_on(self.client.get_all_providers())
    }

    pub fn get_recent_models(&self) -> anyhow::Result<Vec<RecentModelEntry>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_get_recent_models(Arc::clone(ls)));
        }
        self.block_on(self.client.get_recent_models())
    }

    pub fn put_recent_models(&self, entries: Vec<RecentModelEntry>) -> anyhow::Result<Vec<RecentModelEntry>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_put_recent_models(Arc::clone(ls), entries));
        }
        self.block_on(self.client.put_recent_models(&entries))
    }

    // ── Provider 管理 ──

    pub fn get_provider_descriptor(&self, provider_id: &str) -> anyhow::Result<agendao_client::ProviderDescriptorResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_get_provider_descriptor(Arc::clone(ls), provider_id));
        }
        self.block_on(self.client.get_provider_descriptor(provider_id))
    }

    pub fn connect_provider(&self, provider_id: &str, api_key: &str, base_url: Option<String>, protocol: Option<String>) -> anyhow::Result<()> {
        if let Some(ref ls) = self.local {
            use agendao_client::ConnectProviderRequest;
            let req = ConnectProviderRequest {
                provider_id: provider_id.to_string(),
                api_key: api_key.to_string(),
                base_url,
                protocol,
            };
            return self.block_on(agendao_server_local::local_connect_provider(Arc::clone(ls), req));
        }
        self.block_on(self.client.connect_provider(provider_id, api_key, base_url, protocol))
    }

    pub fn set_auth(&self, provider_id: &str, api_key: &str) -> anyhow::Result<()> {
        self.block_on(self.client.set_auth(provider_id, api_key))
    }

    pub fn register_custom_provider(&self, provider_id: &str, base_url: &str, protocol: &str, api_key: &str) -> anyhow::Result<()> {
        self.block_on(self.client.register_custom_provider(provider_id, base_url, protocol, api_key))
    }

    pub fn get_workspace_context(&self) -> anyhow::Result<agendao_runtime_context::ResolvedWorkspaceContext> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_get_workspace_context(Arc::clone(ls)));
        }
        self.block_on(self.client.get_workspace_context())
    }

    pub fn get_config(&self) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_get_config(Arc::clone(ls)));
        }
        self.block_on(self.client.get_config())
    }

    pub fn refresh_provider_catalog(&self) -> anyhow::Result<agendao_client::RefreshProviderCatalogResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_refresh_provider_catalog(Arc::clone(ls)));
        }
        self.block_on(self.client.refresh_provider_catalog())
    }

    // ── Agents & Modes ──

    pub fn list_agents(&self) -> anyhow::Result<Vec<agendao_client::AgentInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_list_agents(Arc::clone(ls)));
        }
        self.block_on(self.client.list_agents())
    }

    // ── 运行控制 ──

    pub fn abort_session(&self, session_id: &str) -> anyhow::Result<serde_json::Value> {
        self.block_on(self.client.abort_session(session_id))
    }

    pub fn cancel_tool_call(&self, session_id: &str, tool_call_id: &str) -> anyhow::Result<serde_json::Value> {
        self.block_on(self.client.cancel_tool_call(session_id, tool_call_id))
    }

    pub fn execute_shell(&self, session_id: &str, command: String, workdir: Option<String>) -> anyhow::Result<serde_json::Value> {
        self.block_on(self.client.execute_shell(session_id, command, workdir))
    }

    // ── 会话管理 ──

    pub fn fork_session(&self, session_id: &str, message_id: Option<&str>) -> anyhow::Result<agendao_client::SessionInfo> {
        self.block_on(self.client.fork_session(session_id, message_id))
    }

    pub fn share_session(&self, session_id: &str) -> anyhow::Result<agendao_client::ShareResponse> {
        self.block_on(self.client.share_session(session_id))
    }

    pub fn update_session_title(&self, session_id: &str, title: &str) -> anyhow::Result<agendao_client::SessionInfo> {
        self.block_on(self.client.update_session_title(session_id, title))
    }

    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_delete_session(Arc::clone(ls), session_id));
        }
        self.block_on(self.client.delete_session(session_id))
    }

    pub fn reply_question(&self, question_id: &str, answers: Vec<Vec<String>>) -> anyhow::Result<()> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_reply_question(Arc::clone(ls), question_id, answers));
        }
        self.block_on(self.client.reply_question(question_id, answers))
    }

    pub fn reply_permission(&self, permission_id: &str, reply: &str, msg: Option<String>) -> anyhow::Result<()> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_reply_permission(Arc::clone(ls), permission_id, reply.to_string(), msg));
        }
        self.block_on(self.client.reply_permission(permission_id, reply, msg))
    }

    pub fn get_session_todos(&self, session_id: &str) -> anyhow::Result<Vec<agendao_client::ApiTodoItem>> {
        if let Some(ref ls) = self.local {
            let todos = self.block_on(agendao_server_local::local_get_session_todos(Arc::clone(ls), session_id))?;
            return Ok(todos.into_iter().map(|t| agendao_client::ApiTodoItem {
                id: t.id, content: t.content, status: t.status, priority: t.priority,
            }).collect());
        }
        self.block_on(self.client.get_session_todos(session_id))
    }

    pub fn list_execution_modes(&self) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server_local::local_list_execution_modes(Arc::clone(ls)));
        }
        self.block_on(self.client.list_execution_modes())
    }

    // ── Info ──

    pub fn base_url(&self) -> &str { self.client.base_url() }
    pub fn handle(&self) -> &tokio::runtime::Handle { &self.handle }
    pub fn raw_client(&self) -> &AsyncApiClient { &self.client }
}

/// Sort sessions descending by `time.updated` (most recent first).
///
/// Stable to preserve server-provided ordering for ties. Public so other
/// adapter layers can apply the same convention without re-implementing it.
pub fn sort_sessions_recent_first(items: &mut [agendao_client::SessionListItem]) {
    items.sort_by(|a, b| b.time.updated.cmp(&a.time.updated));
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::SessionTime;

    fn make_item(id: &str, updated: i64) -> agendao_client::SessionListItem {
        agendao_client::SessionListItem {
            id: id.to_string(),
            slug: id.to_string(),
            project_id: "p".to_string(),
            directory: "/d".to_string(),
            parent_id: None,
            title: id.to_string(),
            version: "v".to_string(),
            time: SessionTime { created: 0, updated, compacting: None, archived: None },
            summary: None,
            hints: None,
            pending_command_invocation: None,
        }
    }

    #[test]
    fn sort_descending_by_updated() {
        let mut items = vec![
            make_item("a", 100),
            make_item("b", 300),
            make_item("c", 200),
        ];
        sort_sessions_recent_first(&mut items);
        let order: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(order, vec!["b", "c", "a"]);
    }

    #[test]
    fn sort_is_stable_for_ties() {
        let mut items = vec![
            make_item("first", 100),
            make_item("second", 100),
            make_item("third", 100),
        ];
        sort_sessions_recent_first(&mut items);
        let order: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(order, vec!["first", "second", "third"]);
    }
}
