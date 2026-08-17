//! Synchronous API bridge.
//!
//! Wraps `agendao_client::AsyncApiClient` using a background tokio runtime.
//! Mirrors the full API surface of old TUI's RuntimeApiClient.

use agendao_client::transport::{PromptOptions, UnixSocketTransport};
use agendao_client::{AsyncApiClient, PromptResponse, SessionInfo};
use agendao_orchestrator::selector::SchedulerChoice;
use agendao_server::ServerState;
use agendao_state::RecentModelEntry;
use std::sync::Arc;

#[derive(Clone)]
pub struct ApiBridge {
    client: Arc<AsyncApiClient>,
    /// In-process local server state for local-direct mode.
    local: Option<Arc<ServerState>>,
    /// Unix JSON-RPC authority for socket mode.
    unix: Option<Arc<UnixSocketTransport>>,
    handle: tokio::runtime::Handle,
}

impl ApiBridge {
    /// Create an HTTP-based bridge (connects to external server).
    pub fn new(base_url: &str, handle: tokio::runtime::Handle) -> anyhow::Result<Self> {
        Self::new_with_password(base_url, None, handle)
    }

    /// Create an HTTP-based bridge authenticated with the server password.
    pub fn new_with_password(
        base_url: &str,
        server_password: Option<String>,
        handle: tokio::runtime::Handle,
    ) -> anyhow::Result<Self> {
        let client = Arc::new(AsyncApiClient::new_with_password(
            base_url.to_string(),
            server_password,
        ));
        Ok(Self {
            client,
            local: None,
            unix: None,
            handle,
        })
    }

    pub fn new_unix(socket_path: String, handle: tokio::runtime::Handle) -> Self {
        Self {
            client: Arc::new(AsyncApiClient::new("http://127.0.0.1:0".into())),
            local: None,
            unix: Some(Arc::new(UnixSocketTransport::new(socket_path))),
            handle,
        }
    }

    /// Create an in-process local transport backed by the canonical server.
    pub fn new_local(local: Arc<ServerState>, handle: tokio::runtime::Handle) -> Self {
        // AsyncApiClient still created for methods without local_* counterpart;
        // they will error at runtime but will never be called in normal flow.
        let client = Arc::new(AsyncApiClient::new("http://127.0.0.1:0".into()));
        Self {
            client,
            local: Some(local),
            unix: None,
            handle,
        }
    }

    fn block_on<R>(&self, fut: impl std::future::Future<Output = R>) -> R {
        self.handle.block_on(fut)
    }

    // ── Sessions ──

    pub fn create_session(
        &self,
        scheduler: Option<SchedulerChoice>,
        directory: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        if let Some(ref ls) = self.local {
            use agendao_client::CreateSessionRequest;
            let req = CreateSessionRequest {
                scheduler,
                directory,
                project_id: None,
                title: None,
            };
            let result =
                self.block_on(agendao_server::local_create_session(Arc::clone(ls), req))?;
            return Ok(result);
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.create_session(agendao_client::CreateSessionRequest {
                scheduler,
                directory,
                project_id: None,
                title: None,
            }));
        }
        self.block_on(self.client.create_session(scheduler, directory))
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
            self.block_on(agendao_server::local_list_sessions(
                Arc::clone(ls),
                directory.clone(),
                None,
                None,
            ))?
        } else if let Some(ref unix) = self.unix {
            self.block_on(unix.list_sessions_filtered(directory.as_deref(), None, None))?
        } else {
            self.block_on(
                self.client
                    .list_sessions_in_directory(directory.as_deref(), None, None),
            )?
        };
        // Most recent first; ties keep insertion order via sort_by (stable).
        sort_sessions_recent_first(&mut items);
        Ok(items)
    }

    pub fn get_session(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_session(
                Arc::clone(ls),
                session_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_session_info(session_id));
        }
        self.block_on(self.client.get_session(session_id))
    }

    /// 异步变体（U6③ open_session 后台拉取）。
    pub async fn get_session_async(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_get_session(Arc::clone(ls), session_id).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.get_session_info(session_id).await;
        }
        self.client.get_session(session_id).await
    }

    pub fn get_messages(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_list_messages(
                Arc::clone(ls),
                session_id,
                None,
                None,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.list_messages(session_id));
        }
        self.block_on(self.client.get_messages(session_id))
    }

    /// 异步变体（U6③）。
    pub async fn get_messages_async(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_list_messages(Arc::clone(ls), session_id, None, None)
                .await;
        }
        if let Some(ref unix) = self.unix {
            return unix.list_messages(session_id).await;
        }
        self.client.get_messages(session_id).await
    }

    /// 异步变体（U6③）。
    pub async fn get_session_todos_async(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<agendao_client::ApiTodoItem>> {
        if let Some(ref ls) = self.local {
            let todos = agendao_server::local_get_session_todos(Arc::clone(ls), session_id).await?;
            return Ok(todos
                .into_iter()
                .map(|t| agendao_client::ApiTodoItem {
                    id: t.id,
                    content: t.content,
                    status: t.status,
                    priority: t.priority,
                })
                .collect());
        }
        if let Some(ref unix) = self.unix {
            return unix.get_session_todos(session_id).await;
        }
        self.client.get_session_todos(session_id).await
    }

    /// 异步变体（U6③）。
    pub async fn list_questions_async(&self) -> anyhow::Result<Vec<agendao_client::QuestionInfo>> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_list_questions(Arc::clone(ls)).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.list_questions().await;
        }
        self.client.list_questions().await
    }

    /// 异步变体（U6③）。
    pub async fn list_permissions_async(
        &self,
    ) -> anyhow::Result<Vec<agendao_client::PermissionRequestInfo>> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_list_permissions(Arc::clone(ls)).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.list_permissions().await;
        }
        self.client.list_permissions().await
    }

    pub fn send_prompt(&self, session_id: &str, content: String) -> anyhow::Result<PromptResponse> {
        self.send_prompt_with(session_id, content, None, None, None, None)
    }

    /// Send a prompt carrying explicit agent/scheduler/model/variant selections.
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
        scheduler: Option<SchedulerChoice>,
        model: Option<String>,
        variant: Option<String>,
    ) -> anyhow::Result<PromptResponse> {
        if let Some(ref unix) = self.unix {
            let response = self.block_on(unix.prompt(
                session_id,
                &content,
                PromptOptions {
                    agent_id: agent,
                    scheduler,
                    model,
                    variant,
                    source_surface: Some(agendao_types::MessageSourceSurface::Tui),
                    ..Default::default()
                },
            ))?;
            return Ok(transport_prompt_response(response));
        }
        let request = agendao_client::PromptRequest {
            message: Some(content),
            parts: None,
            agent,
            scheduler,
            model,
            variant,
            ingress_source: None,
            idempotency_key: None,
            source_origin: None,
            source_surface: None,
            command: None,
            arguments: None,
        };
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_prompt(
                Arc::clone(ls),
                session_id,
                request,
            ));
        }
        let c = Arc::clone(&self.client);
        self.block_on(c.send_prompt(session_id, request))
    }

    /// Async 版 `send_prompt_with` —— dispatch 的后台 task 调用。
    ///
    /// 直接驱动底层 async client / `local_prompt`，不经 `block_on`，因此不会
    /// 阻塞 revue 主事件循环（这是"按 Enter 后画面冻死"的根因）。现有 30+
    /// 同步方法（`block_on` 包装）保持不变，供 session_list / startup 等同步
    /// 调用点继续使用。
    pub async fn send_prompt_with_async(
        &self,
        session_id: &str,
        content: String,
        agent: Option<String>,
        scheduler: Option<SchedulerChoice>,
        model: Option<String>,
        variant: Option<String>,
    ) -> anyhow::Result<PromptResponse> {
        if let Some(ref unix) = self.unix {
            let response = unix
                .prompt(
                    session_id,
                    &content,
                    PromptOptions {
                        agent_id: agent,
                        scheduler,
                        model,
                        variant,
                        source_surface: Some(agendao_types::MessageSourceSurface::Tui),
                        ..Default::default()
                    },
                )
                .await?;
            return Ok(transport_prompt_response(response));
        }
        let request = agendao_client::PromptRequest {
            message: Some(content),
            parts: None,
            agent,
            scheduler,
            model,
            variant,
            ingress_source: None,
            idempotency_key: None,
            source_origin: None,
            source_surface: None,
            command: None,
            arguments: None,
        };
        if let Some(ref ls) = self.local {
            return agendao_server::local_prompt(Arc::clone(ls), session_id, request).await;
        }
        let c = Arc::clone(&self.client);
        c.send_prompt(session_id, request).await
    }

    // ── Models & Providers ──

    pub fn get_all_providers(&self) -> anyhow::Result<agendao_client::FullProviderListResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_all_providers(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_all_providers());
        }
        self.block_on(self.client.get_all_providers())
    }

    pub fn get_recent_models(&self) -> anyhow::Result<Vec<RecentModelEntry>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_recent_models(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_recent_models());
        }
        self.block_on(self.client.get_recent_models())
    }

    pub fn put_recent_models(
        &self,
        entries: Vec<RecentModelEntry>,
    ) -> anyhow::Result<Vec<RecentModelEntry>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_put_recent_models(
                Arc::clone(ls),
                entries,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.put_recent_models(&entries));
        }
        self.block_on(self.client.put_recent_models(&entries))
    }

    /// Async 版 `put_recent_models` —— 模型选中后的持久化走后台 task
    /// （同步版 block_on 在 runtime worker 内会 panic）。
    pub async fn put_recent_models_async(
        &self,
        entries: Vec<RecentModelEntry>,
    ) -> anyhow::Result<Vec<RecentModelEntry>> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_put_recent_models(Arc::clone(ls), entries).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.put_recent_models(&entries).await;
        }
        self.client.put_recent_models(&entries).await
    }

    // ── Provider 管理 ──

    pub fn get_provider_descriptor(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<agendao_client::ProviderDescriptorResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_provider_descriptor(
                Arc::clone(ls),
                provider_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_provider_descriptor(provider_id));
        }
        self.block_on(self.client.get_provider_descriptor(provider_id))
    }

    pub fn connect_provider(
        &self,
        provider_id: &str,
        api_key: &str,
        base_url: Option<String>,
        protocol: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(ref ls) = self.local {
            use agendao_client::ConnectProviderRequest;
            let req = ConnectProviderRequest {
                provider_id: provider_id.to_string(),
                api_key: api_key.to_string(),
                base_url,
                protocol,
            };
            return self.block_on(agendao_server::local_connect_provider(Arc::clone(ls), req));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(
                unix.connect_provider(&agendao_client::ConnectProviderRequest {
                    provider_id: provider_id.to_string(),
                    api_key: api_key.to_string(),
                    base_url,
                    protocol,
                }),
            );
        }
        self.block_on(
            self.client
                .connect_provider(provider_id, api_key, base_url, protocol),
        )
    }

    pub fn set_auth(&self, provider_id: &str, api_key: &str) -> anyhow::Result<()> {
        if let Some(ref unix) = self.unix {
            return self.block_on(
                unix.connect_provider(&agendao_client::ConnectProviderRequest {
                    provider_id: provider_id.to_string(),
                    api_key: api_key.to_string(),
                    base_url: None,
                    protocol: None,
                }),
            );
        }
        self.block_on(self.client.set_auth(provider_id, api_key))
    }

    /// POST `/provider/register`(server 端与 connect_provider 同一 handler),
    /// local-direct 直接短路到 `local_connect_provider`。
    pub fn register_custom_provider(
        &self,
        provider_id: &str,
        base_url: &str,
        protocol: &str,
        api_key: &str,
    ) -> anyhow::Result<()> {
        if let Some(ref ls) = self.local {
            use agendao_client::ConnectProviderRequest;
            let req = ConnectProviderRequest {
                provider_id: provider_id.to_string(),
                api_key: api_key.to_string(),
                base_url: Some(base_url.to_string()),
                protocol: Some(protocol.to_string()),
            };
            return self.block_on(agendao_server::local_connect_provider(Arc::clone(ls), req));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(
                unix.connect_provider(&agendao_client::ConnectProviderRequest {
                    provider_id: provider_id.to_string(),
                    api_key: api_key.to_string(),
                    base_url: Some(base_url.to_string()),
                    protocol: Some(protocol.to_string()),
                }),
            );
        }
        self.block_on(self.client.register_custom_provider(
            provider_id,
            base_url,
            protocol,
            api_key,
        ))
    }

    /// PUT `/provider/{id}`:改 ProviderConfig 的 name/base_url/protocol(不动 api_key)。
    /// api_key 改走 `connect_provider`,两步分离 → TUI Edit dialog 可"只改名字"。
    pub fn update_provider(
        &self,
        provider_id: &str,
        name: Option<&str>,
        base_url: Option<&str>,
        protocol: Option<&str>,
    ) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_update_provider(
                Arc::clone(ls),
                provider_id,
                name.map(str::to_string),
                base_url.map(str::to_string),
                protocol.map(str::to_string),
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.update_provider(provider_id, name, base_url, protocol));
        }
        self.block_on(
            self.client
                .update_provider(provider_id, name, base_url, protocol),
        )
    }

    /// DELETE `/provider/{id}`:删 ProviderConfig + AuthManager 条目(土律·第四条单点权威)。
    pub fn delete_provider(&self, provider_id: &str) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_delete_provider(
                Arc::clone(ls),
                provider_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.delete_provider(provider_id));
        }
        self.block_on(self.client.delete_provider(provider_id))
    }

    /// GET `/config/provider/{id}/models/{key}`:读 raw ModelConfig。
    /// **Edit 模式必走**:server 端 PUT 是整体覆写,半空 ModelConfig 会丢字段
    /// (cost/reasoning/temperature/...)。Edit 前先 GET 原值,合并 form 4 字段后回写
    /// (土律·第十条 可观测性权利)。
    pub fn get_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
    ) -> anyhow::Result<agendao_config::ModelConfig> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_provider_model_config(
                Arc::clone(ls),
                provider_id,
                model_key,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_provider_model_config(provider_id, model_key));
        }
        self.block_on(
            self.client
                .get_provider_model_config(provider_id, model_key),
        )
    }

    /// PUT `/config/provider/{id}/models/{key}`:写 model config(整体覆写)。
    /// Edit 模式需先 GET raw ModelConfig 合并(prefill)避免半空覆写丢字段。
    pub fn put_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
        model: &agendao_config::ModelConfig,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_put_provider_model_config(
                Arc::clone(ls),
                provider_id,
                model_key,
                model.clone(),
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.put_provider_model_config(provider_id, model_key, model));
        }
        self.block_on(
            self.client
                .put_provider_model_config(provider_id, model_key, model),
        )
    }

    /// DELETE `/config/provider/{id}/models/{key}`:删 model config 条目。
    pub fn delete_provider_model_config(
        &self,
        provider_id: &str,
        model_key: &str,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_delete_provider_model_config(
                Arc::clone(ls),
                provider_id,
                model_key,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.delete_provider_model_config(provider_id, model_key));
        }
        self.block_on(
            self.client
                .delete_provider_model_config(provider_id, model_key),
        )
    }

    pub fn get_workspace_context(
        &self,
    ) -> anyhow::Result<agendao_runtime_context::ResolvedWorkspaceContext> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_workspace_context(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_workspace_context());
        }
        self.block_on(self.client.get_workspace_context())
    }

    pub fn get_config(&self) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_config(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_config());
        }
        self.block_on(self.client.get_config())
    }

    /// 异步 PATCH `/config`（fire-and-forget 持久化用，不阻塞事件循环）。
    pub async fn patch_config_async(
        &self,
        patch: serde_json::Value,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_patch_config(Arc::clone(ls), patch).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.patch_config(&patch).await;
        }
        self.client.patch_config(&patch).await
    }

    /// PATCH `/config`（双模式）：UI 偏好（如 theme）落盘通道。
    pub fn patch_config(&self, patch: serde_json::Value) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_patch_config(Arc::clone(ls), patch));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.patch_config(&patch));
        }
        self.block_on(self.client.patch_config(&patch))
    }

    /// PUT `/provider/{id}/disabled`（双模式）：enable/disable 切换。
    pub fn set_provider_disabled(&self, provider_id: &str, disabled: bool) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_set_provider_disabled(
                Arc::clone(ls),
                provider_id,
                disabled,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.set_provider_disabled(provider_id, disabled));
        }
        self.block_on(self.client.set_provider_disabled(provider_id, disabled))
    }

    /// POST `/provider/{id}/test`（双模式）：测试连接（只读探测）。
    pub fn test_provider_connection(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<agendao_client::TestProviderConnectionResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_test_provider_connection(
                Arc::clone(ls),
                provider_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.test_provider_connection(provider_id));
        }
        self.block_on(self.client.test_provider_connection(provider_id))
    }

    /// 异步变体（U6）：直接 await 底层调用，不经 `block_on`——
    /// 供 settings 测连接的后台 task 使用（同步版在 runtime worker
    /// 内会 panic，与 `send_prompt_with_async` 同构）。
    pub async fn test_provider_connection_async(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<agendao_client::TestProviderConnectionResponse> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_test_provider_connection(Arc::clone(ls), provider_id)
                .await;
        }
        if let Some(ref unix) = self.unix {
            return unix.test_provider_connection(provider_id).await;
        }
        self.client.test_provider_connection(provider_id).await
    }

    /// GET `/session/{id}/runtime`（双模式）：运行时状态（含 usage——
    /// 打开会话时给 token_usage/context 信息条播种,不等下一次投影）。
    pub fn get_session_runtime(
        &self,
        session_id: &str,
    ) -> anyhow::Result<agendao_client::SessionRuntimeState> {
        if let Some(ref ls) = self.local {
            let runtime = self.block_on(agendao_server::local_get_session_runtime(
                Arc::clone(ls),
                session_id,
            ))?;
            return Ok(serde_json::from_value(serde_json::to_value(runtime)?)?);
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_session_runtime(session_id));
        }
        self.block_on(self.client.get_session_runtime(session_id))
    }

    pub fn refresh_provider_catalog(
        &self,
    ) -> anyhow::Result<agendao_client::RefreshProviderCatalogResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_refresh_provider_catalog(Arc::clone(
                ls,
            )));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.refresh_provider_catalog());
        }
        self.block_on(self.client.refresh_provider_catalog())
    }

    // ── Agents & Modes ──

    pub fn list_agents(&self) -> anyhow::Result<Vec<agendao_client::AgentInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_list_agents(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.list_agents());
        }
        self.block_on(self.client.list_agents())
    }

    // ── 运行控制 ──

    pub fn abort_session(&self, session_id: &str) -> anyhow::Result<serde_json::Value> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_abort_session(
                Arc::clone(ls),
                session_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.abort_session(session_id));
        }
        self.block_on(self.client.abort_session(session_id))
    }

    pub fn cancel_tool_call(
        &self,
        session_id: &str,
        tool_call_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_cancel_tool_call(
                Arc::clone(ls),
                session_id,
                tool_call_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.cancel_tool_call(session_id, tool_call_id));
        }
        self.block_on(self.client.cancel_tool_call(session_id, tool_call_id))
    }

    pub fn execute_shell(
        &self,
        session_id: &str,
        command: String,
        workdir: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_execute_shell(
                Arc::clone(ls),
                session_id,
                command,
                workdir,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.execute_shell(session_id, &command, workdir.as_deref()));
        }
        self.block_on(self.client.execute_shell(session_id, command, workdir))
    }

    /// Async 版 `execute_shell` —— dispatch_shell 后台 task 调用（同步版
    /// block_on 在 runtime worker 内会 panic，与 send_prompt_with_async 同构）。
    pub async fn execute_shell_async(
        &self,
        session_id: &str,
        command: String,
        workdir: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_execute_shell(
                Arc::clone(ls),
                session_id,
                command,
                workdir,
            )
            .await;
        }
        if let Some(ref unix) = self.unix {
            return unix
                .execute_shell(session_id, &command, workdir.as_deref())
                .await;
        }
        self.client
            .execute_shell(session_id, command, workdir)
            .await
    }

    // ── 会话管理 ──

    pub fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<agendao_client::SessionInfo> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_fork_session(
                Arc::clone(ls),
                session_id,
                message_id.map(str::to_string),
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.fork_session(session_id, message_id));
        }
        self.block_on(self.client.fork_session(session_id, message_id))
    }

    pub fn share_session(&self, session_id: &str) -> anyhow::Result<agendao_client::ShareResponse> {
        if self.local.is_some() || self.unix.is_some() {
            anyhow::bail!("session sharing requires HTTP transport")
        }
        self.block_on(self.client.share_session(session_id))
    }

    /// /unshare：撤销分享链接。与 `share_session` 同范式，仅走 HTTP
    /// （local-direct 暂不短路；与 share 一致）。
    pub fn unshare_session(&self, session_id: &str) -> anyhow::Result<bool> {
        if self.local.is_some() || self.unix.is_some() {
            anyhow::bail!("session unsharing requires HTTP transport")
        }
        self.block_on(self.client.unshare_session(session_id))
    }

    /// /compact：触发会话压缩。focus=None 时 server 用默认压缩策略。
    pub fn compact_session(
        &self,
        session_id: &str,
        focus: Option<&str>,
    ) -> anyhow::Result<agendao_client::CompactResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_compact_session(
                Arc::clone(ls),
                session_id,
                focus.map(str::to_string),
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.compact_session(session_id, focus));
        }
        self.block_on(self.client.compact_session(session_id, focus))
    }

    /// 异步变体（U6）：/compact 触发可耗数秒，后台 task 用，
    /// 不经 `block_on`。
    pub async fn compact_session_async(
        &self,
        session_id: &str,
        focus: Option<&str>,
    ) -> anyhow::Result<agendao_client::CompactResponse> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_compact_session(
                Arc::clone(ls),
                session_id,
                focus.map(str::to_string),
            )
            .await;
        }
        if let Some(ref unix) = self.unix {
            return unix.compact_session(session_id, focus).await;
        }
        self.client.compact_session(session_id, focus).await
    }

    /// /skill/catalog：列出可用 skills（read-only 视图）。
    pub fn list_skills(
        &self,
        query: Option<&agendao_client::SkillCatalogQuery>,
    ) -> anyhow::Result<Vec<agendao_client::SkillCatalogEntry>> {
        if let Some(ref ls) = self.local {
            let query = query.cloned().unwrap_or_default();
            return self.block_on(agendao_server::local_list_skills(Arc::clone(ls), query));
        }
        if let Some(ref unix) = self.unix {
            let query = query.cloned().unwrap_or_default();
            return self.block_on(unix.list_skills(&query));
        }
        self.block_on(self.client.list_skills(query))
    }

    /// /skill/detail：单个 skill 详情（meta + 正文 content + 来源/可写标记）。
    /// 读视图——skill_list Enter 详情 panel 的数据源（双模式）。
    pub fn get_skill_detail(
        &self,
        name: &str,
    ) -> anyhow::Result<agendao_client::SkillDetailResponse> {
        if let Some(ref ls) = self.local {
            let query = agendao_client::SkillDetailQuery {
                name: name.to_string(),
                ..Default::default()
            };
            return self.block_on(agendao_server::local_get_skill_detail(
                Arc::clone(ls),
                query,
            ));
        }
        let query = agendao_client::SkillDetailQuery {
            name: name.to_string(),
            ..Default::default()
        };
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_skill_detail(&query));
        }
        self.block_on(self.client.get_skill_detail(&query))
    }

    /// /tool/catalog：列出全部 tool（含 disabled，打标）——Settings→Tools 读面。
    pub fn list_tools(&self) -> anyhow::Result<Vec<agendao_client::ToolListEntry>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_list_tools(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.list_tools());
        }
        self.block_on(self.client.list_tools())
    }

    /// PUT `/config/disabled`：整体替换 `disabled_tools` / `skills.disabled`。
    /// `Some(vec)` 允许空 vec 清空（patch merge 表达不了清空）；`None` 不动。
    /// server 侧在 tools 变更时重建 tool registry（即时生效）。
    pub fn put_disabled_config(
        &self,
        update: &agendao_client::DisabledConfigUpdate,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_put_disabled_config(
                Arc::clone(ls),
                update.clone(),
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.put_disabled_config(update));
        }
        self.block_on(self.client.put_disabled_config(update))
    }

    /// POST `/skill/manage`（双模式）：skills 管理写面（本阶段用于 Delete）。
    /// local-direct 短路 `local_manage_skill`——server 侧跳过交互式权限门
    /// （TUI ConfirmDialog 已是用户确认；同步 block_on 无法响应权限弹窗），
    /// HTTP 模式保持走 POST `/skill/manage` 完整权限流。
    pub fn manage_skill(
        &self,
        req: &agendao_client::SkillManageRequest,
    ) -> anyhow::Result<agendao_client::SkillManageResponse> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_manage_skill(
                Arc::clone(ls),
                req.clone(),
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.manage_skill(req));
        }
        self.block_on(self.client.manage_skill(req))
    }

    /// /skill/proposal：列出自演化提案。status 传 "draft" 看待处理项
    /// （ProposalStatus 为 snake_case：draft/accepted/rejected/superseded/applied，
    /// 传 "pending" 会被 server 400 拒绝）；读视图 first slice——
    /// approve/reject 需 update_skill_proposal_status，留 B 层第三批。
    pub fn list_skill_proposals(
        &self,
        status: &str,
    ) -> anyhow::Result<Vec<agendao_client::SkillEvolutionProposal>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_list_skill_proposals(
                Arc::clone(ls),
                status,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.list_skill_proposals(status));
        }
        self.block_on(self.client.list_skill_proposals(status))
    }

    /// /mcp：列出所有 MCP 服务器状态（全局）。读视图——connect/disconnect
    /// 需独立 dialog + API，留后续。
    pub fn get_mcp_status(&self) -> anyhow::Result<Vec<agendao_client::McpStatusInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_mcp_status(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_mcp_status());
        }
        self.block_on(self.client.get_mcp_status())
    }

    /// /session/{id}/recovery：per-session 恢复协议（actions + checkpoints）。
    pub fn get_session_recovery(
        &self,
        session_id: &str,
    ) -> anyhow::Result<agendao_client::SessionRecoveryProtocol> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_get_session_recovery(
                Arc::clone(ls),
                session_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_session_recovery(session_id));
        }
        self.block_on(self.client.get_session_recovery(session_id))
    }

    /// /skill/proposal/{id}/status POST：approve（"accepted"）/reject（"rejected"）。
    /// 直接执行类——dialog 保持打开，Ok 后由调用方 remove_by_id 回流（水生木）。
    pub fn update_skill_proposal_status(
        &self,
        id: &str,
        status: &str,
    ) -> anyhow::Result<agendao_client::SkillEvolutionProposal> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_update_skill_proposal_status(
                Arc::clone(ls),
                id,
                status,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.update_skill_proposal_status(id, status));
        }
        self.block_on(self.client.update_skill_proposal_status(id, status))
    }

    /// /mcp/{name}/connect POST：连接 MCP server。直接执行类——Ok 后重拉
    /// get_mcp_status 回流（status 字段变化非移除，重拉是唯一权威）。
    pub fn connect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_connect_mcp(Arc::clone(ls), name));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.connect_mcp(name));
        }
        self.block_on(self.client.connect_mcp(name))
    }

    /// /mcp/{name}/disconnect POST：断开 MCP server。直接执行类——Ok 后重拉回流。
    pub fn disconnect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_disconnect_mcp(Arc::clone(ls), name));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.disconnect_mcp(name));
        }
        self.block_on(self.client.disconnect_mcp(name))
    }

    /// /mcp/{name}/auth POST：发起 OAuth，返回授权 URL（双模式）。
    pub fn start_mcp_auth(&self, name: &str) -> anyhow::Result<agendao_client::McpAuthStartInfo> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_start_mcp_auth(Arc::clone(ls), name));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.start_mcp_auth(name));
        }
        self.block_on(self.client.start_mcp_auth(name))
    }

    /// /mcp/{name}/auth/authenticate POST：完成挂起的 OAuth 交换（双模式）。
    /// 用户浏览器授权完成后调用；Ok 后重拉 get_mcp_status 回流。
    pub fn authenticate_mcp(&self, name: &str) -> anyhow::Result<agendao_client::McpStatusInfo> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_authenticate_mcp(Arc::clone(ls), name));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.authenticate_mcp(name));
        }
        self.block_on(self.client.authenticate_mcp(name))
    }

    /// /mcp/{name}/auth DELETE：清除已存 OAuth 凭据（双模式）。
    pub fn remove_mcp_auth(&self, name: &str) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_remove_mcp_auth(Arc::clone(ls), name));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.remove_mcp_auth(name));
        }
        self.block_on(self.client.remove_mcp_auth(name))
    }

    /// PUT `/config/mcp/{key}`（双模式）：写 MCP server 配置条目。
    /// 增/改/启停共用——整体覆写语义，Edit 前调用方须先读 config 合并。
    pub fn put_mcp_config(
        &self,
        key: &str,
        mcp: &agendao_config::McpServerConfig,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_put_mcp_config(
                Arc::clone(ls),
                key,
                mcp.clone(),
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.put_mcp_config(key, mcp));
        }
        self.block_on(self.client.put_mcp_config(key, mcp))
    }

    /// DELETE `/config/mcp/{key}`（双模式）：删 MCP server 配置条目。
    pub fn delete_mcp_config(&self, key: &str) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_delete_mcp_config(Arc::clone(ls), key));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.delete_mcp_config(key));
        }
        self.block_on(self.client.delete_mcp_config(key))
    }

    /// GET `/config/plugins`（双模式）：已安装插件列表（managed + discovered）。
    pub fn list_plugins(&self) -> anyhow::Result<Vec<agendao_client::PluginListEntry>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_list_plugins(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.list_plugins());
        }
        self.block_on(self.client.list_plugins())
    }

    /// PUT `/config/plugin/{key}`（双模式）：写 plugin 配置条目（安装）。
    pub fn put_plugin_config(
        &self,
        key: &str,
        plugin: &agendao_config::PluginConfig,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_put_plugin_config(
                Arc::clone(ls),
                key,
                plugin.clone(),
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.put_plugin_config(key, plugin));
        }
        self.block_on(self.client.put_plugin_config(key, plugin))
    }

    /// DELETE `/config/plugin/{key}`（双模式）：删 managed plugin 配置条目。
    pub fn delete_plugin_config(&self, key: &str) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_delete_plugin_config(
                Arc::clone(ls),
                key,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.delete_plugin_config(key));
        }
        self.block_on(self.client.delete_plugin_config(key))
    }

    // ── U6④ settings 写操作异步变体 ──
    // 与同步版同一权威、同一路径分支，仅改驱动方式：直接 await 底层调用，
    // 不经 `block_on`（同步版在 runtime worker 内会 panic）。供 settings
    // 写操作的后台 task 使用（火=spawn，水=Tick drain 回收 SettingsWriteDone）。

    /// 异步 GET `/config`。
    pub async fn get_config_async(&self) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_get_config(Arc::clone(ls)).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.get_config().await;
        }
        self.client.get_config().await
    }

    /// 异步 PUT `/config/disabled`。
    pub async fn put_disabled_config_async(
        &self,
        update: &agendao_client::DisabledConfigUpdate,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_put_disabled_config(Arc::clone(ls), update.clone()).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.put_disabled_config(update).await;
        }
        self.client.put_disabled_config(update).await
    }

    /// 异步 POST `/skill/manage`。
    pub async fn manage_skill_async(
        &self,
        req: &agendao_client::SkillManageRequest,
    ) -> anyhow::Result<agendao_client::SkillManageResponse> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_manage_skill(Arc::clone(ls), req.clone()).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.manage_skill(req).await;
        }
        self.client.manage_skill(req).await
    }

    /// 异步 POST `/skill/proposal/{id}/status`。
    pub async fn update_skill_proposal_status_async(
        &self,
        id: &str,
        status: &str,
    ) -> anyhow::Result<agendao_client::SkillEvolutionProposal> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_update_skill_proposal_status(Arc::clone(ls), id, status)
                .await;
        }
        if let Some(ref unix) = self.unix {
            return unix.update_skill_proposal_status(id, status).await;
        }
        self.client.update_skill_proposal_status(id, status).await
    }

    /// 异步 POST `/mcp/{name}/connect`。
    pub async fn connect_mcp_async(&self, name: &str) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_connect_mcp(Arc::clone(ls), name).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.connect_mcp(name).await;
        }
        self.client.connect_mcp(name).await
    }

    /// 异步 POST `/mcp/{name}/disconnect`。
    pub async fn disconnect_mcp_async(&self, name: &str) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_disconnect_mcp(Arc::clone(ls), name).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.disconnect_mcp(name).await;
        }
        self.client.disconnect_mcp(name).await
    }

    /// 异步 PUT `/config/mcp/{key}`。
    pub async fn put_mcp_config_async(
        &self,
        key: &str,
        mcp: &agendao_config::McpServerConfig,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_put_mcp_config(Arc::clone(ls), key, mcp.clone()).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.put_mcp_config(key, mcp).await;
        }
        self.client.put_mcp_config(key, mcp).await
    }

    /// 异步 DELETE `/config/mcp/{key}`。
    pub async fn delete_mcp_config_async(
        &self,
        key: &str,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_delete_mcp_config(Arc::clone(ls), key).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.delete_mcp_config(key).await;
        }
        self.client.delete_mcp_config(key).await
    }

    /// 异步 PUT `/config/plugin/{key}`。
    pub async fn put_plugin_config_async(
        &self,
        key: &str,
        plugin: &agendao_config::PluginConfig,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_put_plugin_config(Arc::clone(ls), key, plugin.clone())
                .await;
        }
        if let Some(ref unix) = self.unix {
            return unix.put_plugin_config(key, plugin).await;
        }
        self.client.put_plugin_config(key, plugin).await
    }

    /// 异步 DELETE `/config/plugin/{key}`。
    pub async fn delete_plugin_config_async(
        &self,
        key: &str,
    ) -> anyhow::Result<agendao_config::Config> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_delete_plugin_config(Arc::clone(ls), key).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.delete_plugin_config(key).await;
        }
        self.client.delete_plugin_config(key).await
    }

    // ── U6⑤ 弹窗打开拉取异步变体 ──
    // 同一权威、同一分支，仅改驱动方式（直接 await，不经 block_on）。
    // 供 `/sessions` `/skills` `/mcps` 等弹窗打开的后台拉取
    // （火=spawn，水=Tick drain 回收 DialogFetchDone）。

    /// 异步列出目录下会话（保持 recent-first 排序口径）。
    pub async fn list_sessions_in_directory_async(
        &self,
        directory: Option<String>,
    ) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
        let mut items = if let Some(ref ls) = self.local {
            agendao_server::local_list_sessions(Arc::clone(ls), directory.clone(), None, None)
                .await?
        } else if let Some(ref unix) = self.unix {
            unix.list_sessions_filtered(directory.as_deref(), None, None)
                .await?
        } else {
            self.client
                .list_sessions_in_directory(directory.as_deref(), None, None)
                .await?
        };
        sort_sessions_recent_first(&mut items);
        Ok(items)
    }

    /// 异步拉 recent models（ModelSelect "★ Recent" 区块）。
    pub async fn get_recent_models_async(&self) -> anyhow::Result<Vec<RecentModelEntry>> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_get_recent_models(Arc::clone(ls)).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.get_recent_models().await;
        }
        self.client.get_recent_models().await
    }

    /// 异步列 skills catalog。
    pub async fn list_skills_async(
        &self,
        query: Option<&agendao_client::SkillCatalogQuery>,
    ) -> anyhow::Result<Vec<agendao_client::SkillCatalogEntry>> {
        if let Some(ref ls) = self.local {
            let query = query.cloned().unwrap_or_default();
            return agendao_server::local_list_skills(Arc::clone(ls), query).await;
        }
        if let Some(ref unix) = self.unix {
            let query = query.cloned().unwrap_or_default();
            return unix.list_skills(&query).await;
        }
        self.client.list_skills(query).await
    }

    /// 异步列 skill proposals。
    pub async fn list_skill_proposals_async(
        &self,
        status: &str,
    ) -> anyhow::Result<Vec<agendao_client::SkillEvolutionProposal>> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_list_skill_proposals(Arc::clone(ls), status).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.list_skill_proposals(status).await;
        }
        self.client.list_skill_proposals(status).await
    }

    /// 异步拉 MCP 状态列表。
    pub async fn get_mcp_status_async(&self) -> anyhow::Result<Vec<agendao_client::McpStatusInfo>> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_get_mcp_status(Arc::clone(ls)).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.get_mcp_status().await;
        }
        self.client.get_mcp_status().await
    }

    /// 异步拉 per-session recovery 协议。
    pub async fn get_session_recovery_async(
        &self,
        session_id: &str,
    ) -> anyhow::Result<agendao_client::SessionRecoveryProtocol> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_get_session_recovery(Arc::clone(ls), session_id).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.get_session_recovery(session_id).await;
        }
        self.client.get_session_recovery(session_id).await
    }

    /// 异步列 execution modes。
    pub async fn list_execution_modes_async(
        &self,
    ) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
        if let Some(ref ls) = self.local {
            return agendao_server::local_list_execution_modes(Arc::clone(ls)).await;
        }
        if let Some(ref unix) = self.unix {
            return unix.list_execution_modes().await;
        }
        self.client.list_execution_modes().await
    }

    /// /session/{id}/recovery/execute POST：执行 recovery action。confirm 类——
    /// 经 PendingConfirm::ExecuteRecovery 路由（panel_dispatch），不在 list dialog 直接调。
    pub fn execute_session_recovery(
        &self,
        session_id: &str,
        action: agendao_client::RecoveryActionKind,
    ) -> anyhow::Result<serde_json::Value> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_execute_session_recovery(
                Arc::clone(ls),
                session_id,
                action,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.execute_session_recovery(session_id, action));
        }
        self.block_on(self.client.execute_session_recovery(session_id, action))
    }

    pub fn update_session_title(
        &self,
        session_id: &str,
        title: &str,
    ) -> anyhow::Result<agendao_client::SessionInfo> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_update_session_title(
                Arc::clone(ls),
                session_id,
                title,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.update_session_title(session_id, title));
        }
        self.block_on(self.client.update_session_title(session_id, title))
    }

    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<bool> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_delete_session(
                Arc::clone(ls),
                session_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.delete_session(session_id));
        }
        self.block_on(self.client.delete_session(session_id))
    }

    pub fn reply_question(
        &self,
        question_id: &str,
        answers: Vec<Vec<String>>,
    ) -> anyhow::Result<()> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_reply_question(
                Arc::clone(ls),
                question_id,
                answers,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.reply_question(question_id, answers));
        }
        self.block_on(self.client.reply_question(question_id, answers))
    }

    /// GET `/question`（双模式）：pending 问题 catch-up（F4）——
    /// 订阅建立前已存在的提问，事件流不重放，打开会话时拉一次合并。
    pub fn list_questions(&self) -> anyhow::Result<Vec<agendao_client::QuestionInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_list_questions(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.list_questions());
        }
        self.block_on(self.client.list_questions())
    }

    /// GET `/permission`（双模式）：pending 权限请求 catch-up（F4）。
    pub fn list_permissions(&self) -> anyhow::Result<Vec<agendao_client::PermissionRequestInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_list_permissions(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.list_permissions());
        }
        self.block_on(self.client.list_permissions())
    }

    /// DELETE `/question/{id}`（双模式）：驳回一个提问（放弃等待）。
    pub fn reject_question(&self, question_id: &str) -> anyhow::Result<()> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_reject_question(
                Arc::clone(ls),
                question_id,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.reject_question(question_id));
        }
        self.block_on(self.client.reject_question(question_id))
    }

    pub fn reply_permission(
        &self,
        permission_id: &str,
        reply: &str,
        msg: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_reply_permission(
                Arc::clone(ls),
                permission_id,
                reply.to_string(),
                msg,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.reply_permission(permission_id, reply, msg));
        }
        self.block_on(self.client.reply_permission(permission_id, reply, msg))
    }

    pub fn set_session_permission_mode(
        &self,
        session_id: &str,
        mode: agendao_client::SessionPermissionMode,
    ) -> anyhow::Result<agendao_client::SessionInfo> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_set_session_permission_mode(
                Arc::clone(ls),
                session_id,
                mode,
            ));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.set_session_permission_mode(session_id, mode));
        }
        self.block_on(self.client.set_session_permission_mode(session_id, mode))
    }

    pub fn get_session_todos(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<agendao_client::ApiTodoItem>> {
        if let Some(ref ls) = self.local {
            let todos = self.block_on(agendao_server::local_get_session_todos(
                Arc::clone(ls),
                session_id,
            ))?;
            return Ok(todos
                .into_iter()
                .map(|t| agendao_client::ApiTodoItem {
                    id: t.id,
                    content: t.content,
                    status: t.status,
                    priority: t.priority,
                })
                .collect());
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.get_session_todos(session_id));
        }
        self.block_on(self.client.get_session_todos(session_id))
    }

    pub fn list_execution_modes(&self) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
        if let Some(ref ls) = self.local {
            return self.block_on(agendao_server::local_list_execution_modes(Arc::clone(ls)));
        }
        if let Some(ref unix) = self.unix {
            return self.block_on(unix.list_execution_modes());
        }
        self.block_on(self.client.list_execution_modes())
    }

    // ── Info ──

    pub fn base_url(&self) -> &str {
        self.client.base_url()
    }
    pub fn handle(&self) -> &tokio::runtime::Handle {
        &self.handle
    }
    pub fn raw_client(&self) -> &AsyncApiClient {
        &self.client
    }
}

fn transport_prompt_response(
    response: agendao_client::transport::PromptResponse,
) -> PromptResponse {
    PromptResponse {
        status: response.text,
        ok: Some(true),
        session_id: Some(response.session_id),
        queued_count: None,
        pending_question_id: None,
        command: None,
        missing_fields: Vec::new(),
    }
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
            time: SessionTime {
                created: 0,
                updated,
                compacting: None,
                archived: None,
            },
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
