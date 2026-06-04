use agendao_execution_types::{CompiledExecutionRequest, ExecutionRequestContext};
use agendao_permission::{PermissionClass, PermissionLifetime, PermissionMatcherKind};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "lsp")]
use agendao_lsp::LspClientRegistry;

pub type Metadata = HashMap<String, serde_json::Value>;

type FileLock = Arc<Mutex<()>>;
type FileLockMap = HashMap<String, FileLock>;
type SharedFileLockMap = Arc<std::sync::Mutex<FileLockMap>>;

static FILE_LOCKS: std::sync::OnceLock<SharedFileLockMap> = std::sync::OnceLock::new();

fn get_file_locks() -> SharedFileLockMap {
    FILE_LOCKS
        .get_or_init(|| Arc::new(std::sync::Mutex::new(HashMap::new())))
        .clone()
}

pub async fn with_file_lock<F, Fut, T>(filepath: &str, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let lock = {
        let locks = get_file_locks();
        let mut locks_guard = locks.lock().unwrap();
        locks_guard
            .entry(filepath.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };

    let _guard = lock.lock().await;
    f().await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSchemaSourceKind {
    BuiltIn,
    Mcp,
    Plugin,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionDef {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multiple: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

pub type AskCallback = Arc<
    dyn (Fn(
            PermissionRequest,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type QuestionCallback = Arc<
    dyn (Fn(
            Vec<QuestionDef>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<Vec<String>>, ToolError>> + Send>,
        >) + Send
        + Sync,
>;

pub type SwitchAgentCallback = Arc<
    dyn (Fn(
            String,
            Option<String>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type CreateSubsessionCallback = Arc<
    dyn (Fn(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type PromptSubsessionCallback = Arc<
    dyn (Fn(
            String,
            agendao_types::SubsessionHandoffPacket,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<agendao_types::SubsessionResultEnvelope, ToolError>,
                    > + Send,
            >,
        >) + Send
        + Sync,
>;

pub type BuildAgentCallback = Arc<
    dyn (Fn(
            String,
            Option<String>,
            Option<String>,
            Option<u32>,
            Vec<String>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<TaskAgentInfo, ToolError>> + Send>,
        >) + Send
        + Sync,
>;

pub type FileTimeAssertCallback = Arc<
    dyn (Fn(
            String,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type FileTimeReadCallback = Arc<
    dyn (Fn(
            String,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type PublishBusCallback = Arc<
    dyn (Fn(
            String,
            serde_json::Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>)
        + Send
        + Sync,
>;

pub type UpdatePartCallback = Arc<
    dyn (Fn(
            serde_json::Value,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type UpdateMessageCallback = Arc<
    dyn (Fn(
            serde_json::Value,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type LspTouchFileCallback = Arc<
    dyn (Fn(
            String,
            bool,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyntheticAttachment {
    pub url: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItemData {
    pub content: String,
    pub status: String,
    pub priority: String,
}

pub type TodoUpdateCallback = Arc<
    dyn (Fn(
            String,
            Vec<TodoItemData>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type TodoGetCallback = Arc<
    dyn (Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<TodoItemData>, ToolError>> + Send>,
        >) + Send
        + Sync,
>;

pub type GetLastModelCallback = Arc<
    dyn (Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<String>, ToolError>> + Send>,
        >) + Send
        + Sync,
>;

#[derive(Debug, Clone)]
pub struct TaskAgentInfo {
    pub name: String,
    pub model: Option<TaskAgentModel>,
    pub can_use_task: bool,
    pub steps: Option<u32>,
    pub execution: Option<ExecutionRequestContext>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub variant: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TaskAgentModel {
    pub provider_id: String,
    pub model_id: String,
}

impl TaskAgentInfo {
    pub fn execution_model(&self) -> Option<TaskAgentModel> {
        self.execution
            .as_ref()
            .and_then(|context| context.model_ref())
            .map(|model| TaskAgentModel {
                provider_id: model.provider_id,
                model_id: model.model_id,
            })
            .or_else(|| self.model.clone())
    }

    pub fn compiled_request(&self) -> Option<CompiledExecutionRequest> {
        self.execution
            .as_ref()
            .and_then(|context| context.compile())
            .or_else(|| {
                self.execution_model()
                    .map(|model| CompiledExecutionRequest {
                        model_id: model.model_id,
                        max_tokens: self.max_tokens,
                        temperature: self.temperature,
                        top_p: self.top_p,
                        variant: self.variant.clone(),
                        provider_options: None,
                    })
            })
    }
}

pub type GetAgentInfoCallback = Arc<
    dyn (Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<TaskAgentInfo>, ToolError>> + Send>,
        >) + Send
        + Sync,
>;

#[derive(Debug, Clone)]
pub struct TaskCategoryInfo {
    pub name: String,
    pub description: String,
    pub model: Option<TaskAgentModel>,
    pub prompt_suffix: Option<String>,
    pub variant: Option<String>,
}

pub type ResolveCategoryCallback = Arc<
    dyn (Fn(
            String,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<Option<TaskCategoryInfo>, ToolError>>
                    + Send,
            >,
        >) + Send
        + Sync,
>;

pub type CreateSyntheticMessageCallback = Arc<
    dyn (Fn(
            String,
            Option<String>,
            String,
            Vec<SyntheticAttachment>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub permission: String,
    pub patterns: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub always: Vec<String>,
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
}

impl PermissionRequest {
    pub fn new(permission: impl Into<String>) -> Self {
        let permission = permission.into();
        let permission_class = default_permission_class_for_name(&permission);
        Self {
            origin_tool: Some(permission.clone()),
            permission_class: Some(permission_class),
            permission,
            patterns: Vec::new(),
            metadata: HashMap::new(),
            always: Vec::new(),
            scope_key: None,
            matcher_kind: None,
            matcher_key: None,
            risk_tags: Vec::new(),
            supported_lifetimes: default_supported_lifetimes_for_class(permission_class),
        }
    }

    pub fn with_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.patterns.push(pattern.into());
        self
    }

    pub fn with_patterns(mut self, patterns: Vec<String>) -> Self {
        self.patterns = patterns;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    pub fn with_permission_class(mut self, permission_class: PermissionClass) -> Self {
        self.permission_class = Some(permission_class);
        self.supported_lifetimes = default_supported_lifetimes_for_class(permission_class);
        self
    }

    pub fn with_scope_key(mut self, scope_key: impl Into<String>) -> Self {
        let scope_key = scope_key.into();
        if self.matcher_kind.is_none() {
            self.matcher_kind = Some(PermissionMatcherKind::ScopeOnly);
            self.matcher_key = Some(scope_key.clone());
        }
        self.scope_key = Some(scope_key);
        self
    }

    pub fn with_origin_tool(mut self, origin_tool: impl Into<String>) -> Self {
        self.origin_tool = Some(origin_tool.into());
        self
    }

    pub fn with_matcher(
        mut self,
        matcher_kind: PermissionMatcherKind,
        matcher_key: impl Into<String>,
    ) -> Self {
        self.matcher_kind = Some(matcher_kind);
        self.matcher_key = Some(matcher_key.into());
        self
    }

    pub fn with_risk_tag(mut self, risk_tag: impl Into<String>) -> Self {
        self.risk_tags.push(risk_tag.into());
        self
    }

    pub fn with_supported_lifetimes(
        mut self,
        supported_lifetimes: Vec<PermissionLifetime>,
    ) -> Self {
        self.supported_lifetimes = supported_lifetimes;
        self
    }

    pub fn with_always(mut self, always: impl Into<String>) -> Self {
        self.always.push(always.into());
        self
    }

    pub fn always_allow(mut self) -> Self {
        self.always.push("*".to_string());
        self
    }
}

fn default_permission_class_for_name(permission: &str) -> PermissionClass {
    match permission {
        "read" | "grep" | "glob" | "list" | "lsp" | "repo_history" | "skill" | "context_docs"
        | "media_inspect" | "todoread" => PermissionClass::InspectRead,
        "write" | "edit" | "multiedit" | "apply_patch" | "patch" | "todowrite"
        | "ast_grep_replace" | "skill_manage" => PermissionClass::WorkspaceWrite,
        "external_directory" | "webfetch" | "websearch" | "browser_session" | "github_research"
        | "skill_hub" | "codesearch" => PermissionClass::ExternalAccess,
        "bash" | "shell_session" | "task" | "task_flow" => PermissionClass::DangerousExec,
        _ => PermissionClass::DangerousExec,
    }
}

pub fn default_supported_lifetimes_for_class(
    permission_class: PermissionClass,
) -> Vec<PermissionLifetime> {
    match permission_class {
        PermissionClass::InspectRead => vec![PermissionLifetime::Once],
        PermissionClass::WorkspaceWrite | PermissionClass::ExternalAccess => vec![
            PermissionLifetime::Once,
            PermissionLifetime::Turn,
            PermissionLifetime::Session,
        ],
        PermissionClass::DangerousExec => vec![PermissionLifetime::Once],
    }
}

pub fn structured_dangerous_exec_lifetimes() -> Vec<PermissionLifetime> {
    vec![
        PermissionLifetime::Once,
        PermissionLifetime::Turn,
        PermissionLifetime::Session,
    ]
}

pub fn workspace_scope_key(project_root: &str, path: &str) -> String {
    let project_root = std::path::Path::new(project_root);
    let path = std::path::Path::new(path);
    if let Ok(relative) = path.strip_prefix(project_root) {
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.is_empty() {
            "workspace:/".to_string()
        } else {
            format!("workspace:/{}", relative)
        }
    } else {
        format!("workspace:{}", path.to_string_lossy().replace('\\', "/"))
    }
}

pub fn external_fs_scope_key(path: &str) -> String {
    format!("fs:{}", path.replace('\\', "/"))
}

pub fn network_scope_key(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    Some(format!("net:{host}"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolRuntimeConfig {
    #[serde(
        rename = "contextDocsRegistryPath",
        alias = "context_docs_registry_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_docs_registry_path: Option<String>,
}

pub const SUBSESSION_HANDOFF_RECENT_TAIL_EXTRA_KEY: &str = "subsession_handoff_recent_tail";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum SubsessionHandoffTailExtraEntry {
    Text(String),
    Rich {
        text: String,
        #[serde(default)]
        title: Option<String>,
    },
}

pub fn subsession_handoff_recent_tail_fields(
    extra: &Metadata,
) -> Vec<agendao_types::SubsessionHandoffField> {
    extra
        .get(SUBSESSION_HANDOFF_RECENT_TAIL_EXTRA_KEY)
        .and_then(|value| {
            serde_json::from_value::<Vec<SubsessionHandoffTailExtraEntry>>(value.clone()).ok()
        })
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            SubsessionHandoffTailExtraEntry::Text(text) => {
                let trimmed = text.trim();
                (!trimmed.is_empty()).then(|| {
                    agendao_types::SubsessionHandoffField::new(
                        agendao_types::SubsessionHandoffFieldKind::SanctionedRecentTail,
                        trimmed.to_string(),
                    )
                })
            }
            SubsessionHandoffTailExtraEntry::Rich { text, title } => {
                let trimmed = text.trim();
                (!trimmed.is_empty()).then(|| {
                    if let Some(title) = title
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                    {
                        agendao_types::SubsessionHandoffField::titled(
                            agendao_types::SubsessionHandoffFieldKind::SanctionedRecentTail,
                            title,
                            trimmed.to_string(),
                        )
                    } else {
                        agendao_types::SubsessionHandoffField::new(
                            agendao_types::SubsessionHandoffFieldKind::SanctionedRecentTail,
                            trimmed.to_string(),
                        )
                    }
                })
            }
        })
        .collect()
}

pub fn append_subsession_handoff_recent_tail_from_extra(
    packet: &mut agendao_types::SubsessionHandoffPacket,
    extra: &Metadata,
) {
    let fields = subsession_handoff_recent_tail_fields(extra);
    if fields.is_empty() {
        return;
    }

    packet.richness = agendao_types::SubsessionHandoffRichness::Enriched;
    packet.fields.extend(fields);
}

impl ToolRuntimeConfig {
    pub fn from_config(config: &agendao_config::Config) -> Self {
        Self {
            context_docs_registry_path: config
                .docs
                .as_ref()
                .and_then(|docs| docs.context_docs_registry_path.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub title: String,
    pub output: String,
    pub metadata: Metadata,
    pub truncated: bool,
}

impl ToolResult {
    pub fn simple(title: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            output: output.into(),
            metadata: Metadata::new(),
            truncated: false,
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub message_id: String,
    pub agent: String,
    pub call_id: Option<String>,
    pub directory: String,
    pub worktree: String,
    pub abort: CancellationToken,
    pub extra: HashMap<String, serde_json::Value>,
    pub ask: Option<AskCallback>,
    pub ask_question: Option<QuestionCallback>,
    pub switch_agent: Option<SwitchAgentCallback>,
    pub create_subsession: Option<CreateSubsessionCallback>,
    pub prompt_subsession: Option<PromptSubsessionCallback>,
    pub file_time_assert: Option<FileTimeAssertCallback>,
    pub file_time_read: Option<FileTimeReadCallback>,
    pub publish_bus: Option<PublishBusCallback>,
    pub update_part: Option<UpdatePartCallback>,
    pub update_message: Option<UpdateMessageCallback>,
    pub lsp_touch_file: Option<LspTouchFileCallback>,
    pub todo_update: Option<TodoUpdateCallback>,
    pub todo_get: Option<TodoGetCallback>,
    pub get_last_model: Option<GetLastModelCallback>,
    pub get_agent_info: Option<GetAgentInfoCallback>,
    pub resolve_category: Option<ResolveCategoryCallback>,
    pub build_agent: Option<BuildAgentCallback>,
    pub create_synthetic_message: Option<CreateSyntheticMessageCallback>,
    pub project_root: String,
    pub runtime_config: ToolRuntimeConfig,
    pub config_store: Option<Arc<agendao_config::ConfigStore>>,
    pub registry: Option<Arc<dyn ToolRegistryAccess>>,
    #[cfg(feature = "lsp")]
    pub lsp_registry: Option<Arc<LspClientRegistry>>,
}

#[async_trait]
pub trait ToolRegistryAccess: Send + Sync {
    async fn get(&self, id: &str) -> Option<Arc<dyn Tool>>;
    async fn list_ids(&self) -> Vec<String>;
    async fn suggest_tools(&self, requested: &str) -> Vec<String>;
    async fn execute(
        &self,
        tool_id: &str,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError>;
}

#[async_trait]
impl ToolRegistryAccess for () {
    async fn get(&self, _id: &str) -> Option<Arc<dyn Tool>> {
        None
    }

    async fn list_ids(&self) -> Vec<String> {
        Vec::new()
    }

    async fn suggest_tools(&self, _requested: &str) -> Vec<String> {
        Vec::new()
    }

    async fn execute(
        &self,
        tool_id: &str,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionError(format!(
            "tool registry access not available for `{}`",
            tool_id
        )))
    }
}

impl ToolContext {
    pub fn new(session_id: String, message_id: String, directory: String) -> Self {
        Self {
            session_id,
            message_id,
            agent: String::new(),
            call_id: None,
            directory: directory.clone(),
            worktree: directory.clone(),
            abort: CancellationToken::new(),
            extra: HashMap::new(),
            ask: None,
            ask_question: None,
            switch_agent: None,
            create_subsession: None,
            prompt_subsession: None,
            file_time_assert: None,
            file_time_read: None,
            publish_bus: None,
            update_part: None,
            update_message: None,
            lsp_touch_file: None,
            todo_update: None,
            todo_get: None,
            get_last_model: None,
            get_agent_info: None,
            resolve_category: None,
            build_agent: None,
            create_synthetic_message: None,
            project_root: directory,
            runtime_config: ToolRuntimeConfig::default(),
            config_store: None,
            registry: None,
            #[cfg(feature = "lsp")]
            lsp_registry: None,
        }
    }

    pub fn with_agent(mut self, agent: String) -> Self {
        self.agent = agent;
        self
    }

    pub fn with_abort(mut self, abort: CancellationToken) -> Self {
        self.abort = abort;
        self
    }

    pub fn with_tool_runtime_config(mut self, runtime_config: ToolRuntimeConfig) -> Self {
        self.runtime_config = runtime_config;
        self
    }

    pub fn with_config_store(mut self, config_store: Arc<agendao_config::ConfigStore>) -> Self {
        self.config_store = Some(config_store);
        self
    }

    pub fn with_loaded_config(mut self, config: &agendao_config::Config) -> Self {
        self.runtime_config = ToolRuntimeConfig::from_config(config);
        self
    }

    pub fn with_registry(mut self, registry: Arc<dyn ToolRegistryAccess>) -> Self {
        self.registry = Some(registry);
        self
    }

    #[cfg(feature = "lsp")]
    pub fn with_lsp_registry(mut self, lsp_registry: Arc<LspClientRegistry>) -> Self {
        self.lsp_registry = Some(lsp_registry);
        self
    }

    pub fn with_ask<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(PermissionRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.ask = Some(Arc::new(move |req| Box::pin(callback(req))));
        self
    }

    pub async fn ask_permission(&self, request: PermissionRequest) -> Result<(), ToolError> {
        if let Some(ref callback) = self.ask {
            callback(request).await
        } else {
            Ok(())
        }
    }

    pub fn with_ask_question<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(Vec<QuestionDef>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<Vec<String>>, ToolError>> + Send + 'static,
    {
        self.ask_question = Some(Arc::new(move |questions| Box::pin(callback(questions))));
        self
    }

    pub async fn question(
        &self,
        questions: Vec<QuestionDef>,
    ) -> Result<Vec<Vec<String>>, ToolError> {
        if let Some(ref callback) = self.ask_question {
            callback(questions).await
        } else {
            Err(ToolError::ExecutionError(
                "Question callback not configured".to_string(),
            ))
        }
    }

    pub fn with_switch_agent<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Option<String>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.switch_agent = Some(Arc::new(move |agent, model| {
            Box::pin(callback(agent, model))
        }));
        self
    }

    pub async fn do_switch_agent(
        &self,
        agent: String,
        model: Option<String>,
    ) -> Result<(), ToolError> {
        if let Some(ref callback) = self.switch_agent {
            callback(agent, model).await
        } else {
            Ok(())
        }
    }

    pub fn with_create_subsession<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Option<String>, Option<String>, Vec<String>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, ToolError>> + Send + 'static,
    {
        self.create_subsession = Some(Arc::new(move |agent, title, model, disabled_tools| {
            Box::pin(callback(agent, title, model, disabled_tools))
        }));
        self
    }

    pub async fn do_create_subsession(
        &self,
        agent: String,
        title: Option<String>,
        model: Option<String>,
        disabled_tools: Vec<String>,
    ) -> Result<String, ToolError> {
        if let Some(ref callback) = self.create_subsession {
            callback(agent, title, model, disabled_tools).await
        } else {
            Ok(format!("task_{}_{}", agent, uuid::Uuid::new_v4()))
        }
    }

    pub fn with_prompt_subsession<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, agendao_types::SubsessionHandoffPacket) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<agendao_types::SubsessionResultEnvelope, ToolError>>
            + Send
            + 'static,
    {
        self.prompt_subsession = Some(Arc::new(move |session_id, handoff| {
            Box::pin(callback(session_id, handoff))
        }));
        self
    }

    pub async fn do_prompt_subsession(
        &self,
        session_id: String,
        handoff: agendao_types::SubsessionHandoffPacket,
    ) -> Result<agendao_types::SubsessionResultEnvelope, ToolError> {
        if let Some(ref callback) = self.prompt_subsession {
            callback(session_id, handoff).await
        } else {
            Err(ToolError::ExecutionError(
                "The current execution environment does not support subagent sessions (task/task_flow). \
                 This usually happens when the tool context was created without a prompt_subsession callback."
                    .to_string(),
            ))
        }
    }

    pub fn with_file_time_assert<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.file_time_assert = Some(Arc::new(move |session_id, file_path| {
            Box::pin(callback(session_id, file_path))
        }));
        self
    }

    pub async fn do_file_time_assert(&self, file_path: String) -> Result<(), ToolError> {
        if let Some(ref callback) = self.file_time_assert {
            callback(self.session_id.clone(), file_path).await
        } else {
            Ok(())
        }
    }

    pub fn with_file_time_read<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.file_time_read = Some(Arc::new(move |session_id, file_path| {
            Box::pin(callback(session_id, file_path))
        }));
        self
    }

    pub async fn do_file_time_read(&self, file_path: String) -> Result<(), ToolError> {
        if let Some(ref callback) = self.file_time_read {
            callback(self.session_id.clone(), file_path).await
        } else {
            Ok(())
        }
    }

    pub fn with_publish_bus<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.publish_bus = Some(Arc::new(move |event_type, properties| {
            Box::pin(callback(event_type, properties))
        }));
        self
    }

    pub async fn do_publish_bus(&self, event_type: &str, properties: serde_json::Value) {
        if let Some(ref callback) = self.publish_bus {
            callback(event_type.to_string(), properties).await;
        }
    }

    pub fn with_update_part<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.update_part = Some(Arc::new(move |part| Box::pin(callback(part))));
        self
    }

    pub async fn do_update_part(&self, part: serde_json::Value) -> Result<(), ToolError> {
        if let Some(ref callback) = self.update_part {
            callback(part).await
        } else {
            Ok(())
        }
    }

    pub fn with_update_message<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.update_message = Some(Arc::new(move |msg| Box::pin(callback(msg))));
        self
    }

    pub async fn do_update_message(&self, msg: serde_json::Value) -> Result<(), ToolError> {
        if let Some(ref callback) = self.update_message {
            callback(msg).await
        } else {
            Ok(())
        }
    }

    pub fn with_lsp_touch_file<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, bool) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.lsp_touch_file = Some(Arc::new(move |file_path, write| {
            Box::pin(callback(file_path, write))
        }));
        self
    }

    pub async fn do_lsp_touch_file(&self, file_path: String, write: bool) -> Result<(), ToolError> {
        if let Some(ref callback) = self.lsp_touch_file {
            callback(file_path, write).await
        } else {
            Ok(())
        }
    }

    pub fn with_todo_update<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Vec<TodoItemData>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.todo_update = Some(Arc::new(move |session_id, todos| {
            Box::pin(callback(session_id, todos))
        }));
        self
    }

    pub async fn do_todo_update(&self, todos: Vec<TodoItemData>) -> Result<(), ToolError> {
        if let Some(ref callback) = self.todo_update {
            callback(self.session_id.clone(), todos).await
        } else {
            Ok(())
        }
    }

    pub fn with_todo_get<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<TodoItemData>, ToolError>> + Send + 'static,
    {
        self.todo_get = Some(Arc::new(move |session_id| Box::pin(callback(session_id))));
        self
    }

    pub async fn do_todo_get(&self) -> Result<Vec<TodoItemData>, ToolError> {
        if let Some(ref callback) = self.todo_get {
            callback(self.session_id.clone()).await
        } else {
            Ok(Vec::new())
        }
    }

    pub fn with_get_last_model<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Option<String>, ToolError>> + Send + 'static,
    {
        self.get_last_model = Some(Arc::new(move |session_id| Box::pin(callback(session_id))));
        self
    }

    pub async fn do_get_last_model(&self) -> Option<String> {
        if let Some(ref callback) = self.get_last_model {
            callback(self.session_id.clone()).await.ok().flatten()
        } else {
            None
        }
    }

    pub fn with_get_agent_info<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut:
            std::future::Future<Output = Result<Option<TaskAgentInfo>, ToolError>> + Send + 'static,
    {
        self.get_agent_info = Some(Arc::new(move |name| Box::pin(callback(name))));
        self
    }

    pub async fn do_get_agent_info(&self, name: &str) -> Option<TaskAgentInfo> {
        if let Some(ref callback) = self.get_agent_info {
            callback(name.to_string()).await.ok().flatten()
        } else {
            None
        }
    }

    pub fn with_resolve_category<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Option<TaskCategoryInfo>, ToolError>>
            + Send
            + 'static,
    {
        self.resolve_category = Some(Arc::new(move |cat| Box::pin(callback(cat))));
        self
    }

    pub async fn do_resolve_category(&self, category: &str) -> Option<TaskCategoryInfo> {
        if let Some(ref callback) = self.resolve_category {
            callback(category.to_string()).await.ok().flatten()
        } else {
            None
        }
    }

    pub fn with_build_agent<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Option<String>, Option<String>, Option<u32>, Vec<String>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = Result<TaskAgentInfo, ToolError>> + Send + 'static,
    {
        self.build_agent = Some(Arc::new(move |name, prompt, model, max_steps, tools| {
            Box::pin(callback(name, prompt, model, max_steps, tools))
        }));
        self
    }

    pub async fn do_build_agent(
        &self,
        name: String,
        system_prompt: Option<String>,
        model: Option<String>,
        max_steps: Option<u32>,
        allowed_tools: Vec<String>,
    ) -> Result<TaskAgentInfo, ToolError> {
        if let Some(ref callback) = self.build_agent {
            callback(name, system_prompt, model, max_steps, allowed_tools).await
        } else {
            Err(ToolError::ExecutionError(
                "Build agent callback not configured".to_string(),
            ))
        }
    }

    pub fn with_create_synthetic_message<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Option<String>, String, Vec<SyntheticAttachment>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.create_synthetic_message =
            Some(Arc::new(move |session_id, agent, text, attachments| {
                Box::pin(callback(session_id, agent, text, attachments))
            }));
        self
    }

    pub async fn do_create_synthetic_message(
        &self,
        agent: Option<String>,
        text: String,
    ) -> Result<(), ToolError> {
        self.do_create_synthetic_message_with_attachments(agent, text, Vec::new())
            .await
    }

    pub async fn do_create_synthetic_message_with_attachments(
        &self,
        agent: Option<String>,
        text: String,
        attachments: Vec<SyntheticAttachment>,
    ) -> Result<(), ToolError> {
        if let Some(ref callback) = self.create_synthetic_message {
            callback(self.session_id.clone(), agent, text, attachments).await
        } else {
            Ok(())
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.abort.is_cancelled()
    }

    pub fn is_external_path(&self, path: &str) -> bool {
        let abs_path = if std::path::Path::new(path).is_absolute() {
            path.to_string()
        } else {
            format!("{}/{}", self.directory, path)
        };
        !abs_path.starts_with(&self.project_root)
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("message_id", &self.message_id)
            .field("agent", &self.agent)
            .field("directory", &self.directory)
            .field("worktree", &self.worktree)
            .field("get_agent_info", &self.get_agent_info.is_some())
            .field("resolve_category", &self.resolve_category.is_some())
            .field("build_agent", &self.build_agent.is_some())
            .finish()
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    fn source_kind(&self) -> ToolSchemaSourceKind {
        ToolSchemaSourceKind::BuiltIn
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError>;

    fn validate(&self, args: &serde_json::Value) -> Result<(), ToolError> {
        let _ = args;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Validation error: {0}")]
    ValidationError(String),
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Question rejected: {0}")]
    QuestionRejected(String),
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Binary file: {0}")]
    BinaryFile(String),
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("Cancelled")]
    Cancelled,
}

impl ToolError {
    pub fn with_suggestions(msg: impl Into<String>, suggestions: &[String]) -> Self {
        let msg = msg.into();
        if suggestions.is_empty() {
            ToolError::FileNotFound(msg)
        } else {
            ToolError::FileNotFound(format!(
                "{}\n\nDid you mean one of these?\n{}",
                msg,
                suggestions.join("\n")
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub source_kind: ToolSchemaSourceKind,
}
