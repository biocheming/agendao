use agendao_permission::{PermissionClass, PermissionLifetime, PermissionMatcherKind};
use agendao_types::ToolCatalogMetadata;
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

pub type LspTouchFileCallback = Arc<
    dyn (Fn(
            String,
            bool,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

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
        "bash" | "shell_session" => PermissionClass::DangerousExec,
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
        external_fs_scope_key(&path.to_string_lossy())
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
    pub file_time_assert: Option<FileTimeAssertCallback>,
    pub file_time_read: Option<FileTimeReadCallback>,
    pub publish_bus: Option<PublishBusCallback>,
    pub update_part: Option<UpdatePartCallback>,
    pub lsp_touch_file: Option<LspTouchFileCallback>,
    pub todo_update: Option<TodoUpdateCallback>,
    pub todo_get: Option<TodoGetCallback>,
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
            file_time_assert: None,
            file_time_read: None,
            publish_bus: None,
            update_part: None,
            lsp_touch_file: None,
            todo_update: None,
            todo_get: None,
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
            .finish()
    }
}

/// Cross-call stale-file guard for read→write sequences.
///
/// Read-family tools record a file's mtime after reading; write tools assert
/// the mtime is unchanged before overwriting. Paths that were never read
/// always pass — the guard only protects reads that happened in this
/// process, so fresh writes are never blocked. A passed assert consumes the
/// record: after the first guarded write succeeds, writing the same file
/// again requires a fresh read.
#[derive(Debug, Default)]
pub struct FileTimeTracker {
    mtimes: std::sync::Mutex<HashMap<(String, String), std::time::SystemTime>>,
}

impl FileTimeTracker {
    fn mtime_of(path: &str) -> Result<std::time::SystemTime, ToolError> {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .map_err(|error| {
                ToolError::ExecutionError(format!("Failed to stat file {path}: {error}"))
            })
    }

    /// Record the current mtime of `path` as observed by a read tool.
    pub fn record(&self, session_id: &str, path: &str) -> Result<(), ToolError> {
        let mtime = Self::mtime_of(path)?;
        self.mtimes
            .lock()
            .expect("file-time tracker lock poisoned")
            .insert((session_id.to_string(), path.to_string()), mtime);
        Ok(())
    }

    /// Fail if `path` changed on disk since it was last recorded by a read.
    pub fn assert_unchanged(&self, session_id: &str, path: &str) -> Result<(), ToolError> {
        let key = (session_id.to_string(), path.to_string());
        let recorded = self
            .mtimes
            .lock()
            .expect("file-time tracker lock poisoned")
            .remove(&key);
        let Some(recorded) = recorded else {
            return Ok(());
        };
        let current = Self::mtime_of(path)?;
        if current != recorded {
            return Err(ToolError::ExecutionError(format!(
                "File {path} has been modified since it was last read. Read the file again before writing."
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod file_time_tracker_tests {
    use super::FileTimeTracker;

    fn temp_file(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("agendao_ftt_{name}_{}", std::process::id()));
        std::fs::write(&path, b"one").unwrap();
        path
    }

    #[test]
    fn assert_passes_for_never_read_paths() {
        let tracker = FileTimeTracker::default();
        let path = temp_file("unread");
        assert!(tracker
            .assert_unchanged("ses_1", path.to_str().unwrap())
            .is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn assert_fails_when_mtime_changed_after_read() {
        let tracker = FileTimeTracker::default();
        let path = temp_file("stale");
        let path_str = path.to_str().unwrap();
        tracker.record("ses_1", path_str).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(12345))
            .unwrap();
        let error = tracker
            .assert_unchanged("ses_1", path_str)
            .expect_err("modified file must fail the guard");
        assert!(error
            .to_string()
            .contains("modified since it was last read"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn assert_passes_when_mtime_unchanged_after_read() {
        let tracker = FileTimeTracker::default();
        let path = temp_file("fresh");
        let path_str = path.to_str().unwrap();
        tracker.record("ses_1", path_str).unwrap();
        assert!(tracker.assert_unchanged("ses_1", path_str).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn passed_assert_consumes_the_guard() {
        let tracker = FileTimeTracker::default();
        let path = temp_file("oneshot");
        let path_str = path.to_str().unwrap();
        tracker.record("ses_1", path_str).unwrap();
        assert!(tracker.assert_unchanged("ses_1", path_str).is_ok());
        // Guard consumed: a second write without a fresh read must not fail.
        assert!(tracker.assert_unchanged("ses_1", path_str).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn records_are_scoped_per_session() {
        let tracker = FileTimeTracker::default();
        let path = temp_file("scoped");
        let path_str = path.to_str().unwrap();
        tracker.record("ses_1", path_str).unwrap();
        // Another session never read the file: no guard, no failure.
        assert!(tracker.assert_unchanged("ses_2", path_str).is_ok());
        let _ = std::fs::remove_file(&path);
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
    fn catalog_metadata(&self) -> Option<ToolCatalogMetadata> {
        None
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<ToolCatalogMetadata>,
}
