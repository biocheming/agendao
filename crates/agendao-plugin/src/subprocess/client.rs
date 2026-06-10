//! `PluginSubprocess` — manages a single plugin host child process.
//!
//! Communicates via Content-Length framed JSON-RPC 2.0 over stdin/stdout,
//! mirroring the MCP stdio transport pattern.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

use super::protocol::{RpcError, RpcRequest, RpcResponse};
use super::runtime::JsRuntime;
use agendao_core::codec::{self, CodecError};
use agendao_core::process_registry::{global_registry, ProcessGuard, ProcessKind};
use agendao_core::stderr_drain::{spawn_stderr_drain, StderrDrainConfig};
use agendao_types::ToolCatalogMetadata;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum PluginSubprocessError {
    #[error("subprocess I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("plugin RPC error ({code}): {message}")]
    Rpc { code: i64, message: String },

    #[error("plugin subprocess not running")]
    NotRunning,

    #[error("plugin response timeout")]
    Timeout,

    #[error("protocol error: {0}")]
    Protocol(String),
}

impl From<RpcError> for PluginSubprocessError {
    fn from(e: RpcError) -> Self {
        Self::Rpc {
            code: e.code,
            message: e.message,
        }
    }
}

impl From<CodecError> for PluginSubprocessError {
    fn from(e: CodecError) -> Self {
        match e {
            CodecError::Io(io) => Self::Io(io),
            CodecError::Serialize(se) => Self::Json(se),
            CodecError::Protocol(msg) => Self::Protocol(msg),
            CodecError::ConnectionClosed => Self::NotRunning,
        }
    }
}

// ---------------------------------------------------------------------------
// Result types (deserialized from host responses)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct InitializeResult {
    pub name: String,
    pub hooks: Vec<String>,
    pub auth: Option<AuthMeta>,
    #[serde(default, rename = "pluginID")]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub tools: Option<HashMap<String, PluginToolDef>>,
}

/// Definition of a custom tool registered by a plugin.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginToolDef {
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<ToolCatalogMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthMeta {
    pub provider: String,
    pub methods: Vec<AuthMethodMeta>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthMethodMeta {
    #[serde(rename = "type")]
    pub method_type: String,
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeResult {
    pub url: Option<String>,
    pub instructions: Option<String>,
    pub method: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuthLoadResult {
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(rename = "hasCustomFetch")]
    pub has_custom_fetch: bool,
}

#[derive(Debug, Deserialize)]
pub struct AuthFetchResult {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
}

pub struct AuthFetchStreamResult {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub chunks: mpsc::Receiver<Result<String, PluginSubprocessError>>,
}

// ---------------------------------------------------------------------------
// Context passed to plugin on initialize
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PluginContext {
    pub worktree: String,
    pub directory: String,
    #[serde(rename = "serverUrl")]
    pub server_url: String,
    /// Server-generated token for authenticating internal plugin requests.
    #[serde(
        rename = "internalToken",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub internal_token: String,
}

// ---------------------------------------------------------------------------
// Transport — inner mutable state that gets swapped on reconnect
// ---------------------------------------------------------------------------

/// Payloads larger than this are written to a temp file instead of stdin pipe.
const LARGE_PAYLOAD_THRESHOLD: usize = 64 * 1024; // 64KB

/// IPC temp directory namespaced by PID to avoid conflicts between concurrent instances.
pub(crate) fn ipc_temp_dir() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("agendao-plugin-ipc")
        .join(std::process::id().to_string())
}

struct Transport {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    process: Child,
}

// ---------------------------------------------------------------------------
// PluginSubprocess
// ---------------------------------------------------------------------------

pub struct PluginSubprocess {
    /// Human-readable plugin name (from initialize response).
    name: String,
    /// Stable unique identifier (resolved plugin path), not display name.
    plugin_id: String,
    /// Custom tool definitions registered by this plugin.
    tools: HashMap<String, PluginToolDef>,
    /// Inner transport swapped atomically on reconnect.
    transport: Arc<RwLock<Transport>>,
    /// Serializes RPC call sequences (separate from transport lock).
    rpc_lock: Arc<Mutex<()>>,
    request_id: AtomicU64,
    /// Hook names this plugin registered.
    hooks: Vec<String>,
    /// Auth metadata, if the plugin provides auth.
    auth_meta: Option<AuthMeta>,
    /// RPC call timeout.
    timeout: Duration,
    // -- Saved for reconnect --------------------------------------------------
    runtime: JsRuntime,
    host_script: String,
    plugin_path: String,
    init_context: serde_json::Value,
    cwd: Option<std::path::PathBuf>,
    /// RAII guard — auto-unregisters from ProcessRegistry on drop.
    _process_guard: std::sync::Mutex<Option<ProcessGuard>>,
}

impl PluginSubprocess {
    /// Spawn a plugin host subprocess and run the `initialize` handshake.
    ///
    /// `cwd` sets the working directory for the subprocess so that bare-specifier
    /// `import("pkg")` calls resolve against the correct `node_modules/`.
    pub async fn spawn(
        runtime: JsRuntime,
        host_script: &str,
        plugin_path: &str,
        context: PluginContext,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self, PluginSubprocessError> {
        let args = runtime.run_args(host_script);
        let mut cmd = Command::new(runtime.command());
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Capture stderr so we can log it without corrupting TUI rendering.
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn()?;
        if let Some(stderr) = child.stderr.take() {
            let _handle = spawn_stderr_drain(
                stderr,
                StderrDrainConfig::new(format!("plugin:{}", plugin_path)),
            );
        }

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginSubprocessError::Protocol("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginSubprocessError::Protocol("no stdout".into()))?;

        let init_context = serde_json::to_value(&context)
            .map_err(|e| PluginSubprocessError::Protocol(format!("serialize context: {e}")))?;

        let mut this = Self {
            name: String::new(),
            plugin_id: String::new(),
            tools: HashMap::new(),
            transport: Arc::new(RwLock::new(Transport {
                stdin,
                stdout: BufReader::new(stdout),
                process: child,
            })),
            rpc_lock: Arc::new(Mutex::new(())),
            request_id: AtomicU64::new(1),
            hooks: Vec::new(),
            auth_meta: None,
            timeout: Duration::from_secs(30),
            runtime,
            host_script: host_script.to_string(),
            plugin_path: plugin_path.to_string(),
            init_context,
            cwd: cwd.map(|p| p.to_path_buf()),
            _process_guard: std::sync::Mutex::new(None),
        };

        // Send initialize
        let params = serde_json::json!({
            "pluginPath": plugin_path,
            "context": context,
        });
        let result: InitializeResult = this.call("initialize", Some(params)).await?;

        this.name = result.name;
        this.hooks = result.hooks;
        this.auth_meta = result.auth;
        this.plugin_id = result.plugin_id.unwrap_or_else(|| plugin_path.to_string());
        this.tools = result.tools.unwrap_or_default();

        // Register in global process registry for TUI visibility
        {
            let transport = this.transport.read().await;
            if let Some(pid) = transport.process.id() {
                let guard = global_registry().register_with_shutdown(
                    pid,
                    this.name.clone(),
                    ProcessKind::Plugin,
                    Arc::new({
                        let name = this.name.clone();
                        move || {
                            tracing::debug!(plugin = %name, "Plugin on_shutdown callback fired");
                        }
                    }),
                );
                *this._process_guard.lock().unwrap() = Some(guard);
            }
        }

        Ok(this)
    }

    // -- Accessors ----------------------------------------------------------

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn hooks(&self) -> &[String] {
        &self.hooks
    }

    pub fn auth_meta(&self) -> Option<&AuthMeta> {
        self.auth_meta.as_ref()
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn tools(&self) -> &HashMap<String, PluginToolDef> {
        &self.tools
    }

    // -- RPC methods --------------------------------------------------------

    /// Invoke a custom tool registered by this plugin.
    ///
    /// `on_sent` is called with the RPC request id **after** the request has
    /// been written but **before** the response loop begins.  This is the
    /// correct point to register tracking for cancellation — the tool is now
    /// in-flight and the request id is known.
    pub async fn invoke_tool<F, Fut>(
        &self,
        tool_id: &str,
        args: Value,
        context: Value,
        on_sent: F,
    ) -> Result<Value, PluginSubprocessError>
    where
        F: FnOnce(u64) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let params = serde_json::json!({
            "toolID": tool_id,
            "args": args,
            "context": context,
        });

        // Lock ordering: acquire rpc_lock first, then write, then read.
        // This matches `call()` and prevents concurrent writes from
        // interleaving and losing responses.
        let _rpc_guard = self.rpc_lock.lock().await;
        let id = self.next_id();
        self.write_request_with_timeout(id, "tool.invoke", Some(params))
            .await?;

        // Notify caller so it can register tracking while tool is in-flight.
        on_sent(id).await;

        let mut deadline = tokio::time::Instant::now() + self.timeout;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    if crate::feature_flags::is_enabled("plugin_timeout_self_heal") {
                        if let Err(error) = self.reconnect().await {
                            tracing::debug!(
                                plugin = %self.name,
                                error = %error,
                                "[plugin-heal] reconnect failed after tool timeout"
                            );
                        }
                    }
                    return Err(PluginSubprocessError::Timeout);
                }
                message = self.read_message() => {
                    match message? {
                        super::protocol::RpcMessage::Response(resp) if resp.id == id => {
                            if let Some(err) = resp.error {
                                return Err(err.into());
                            }
                            let result = resp.result.unwrap_or(Value::Null);
                            let output = result.get("output").cloned().unwrap_or(Value::Null);
                            return Ok(output);
                        }
                        ref msg if msg.is_progress_notification() => {
                            deadline = tokio::time::Instant::now() + self.timeout;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Invoke a hook on the plugin.
    pub async fn invoke_hook(
        &self,
        hook: &str,
        input: Value,
        output: Value,
    ) -> Result<Value, PluginSubprocessError> {
        let params = serde_json::json!({
            "hook": hook,
            "input": input,
            "output": output,
        });

        let serialized = serde_json::to_string(&params).unwrap_or_default();
        let payload_bytes = serialized.len();

        tracing::debug!(
            hook = hook,
            plugin = %self.name,
            payload_bytes = payload_bytes,
            "[plugin-perf] invoke_hook payload"
        );

        if payload_bytes > LARGE_PAYLOAD_THRESHOLD
            && crate::feature_flags::is_enabled("plugin_large_payload_file_ipc")
        {
            // Write to temp file for large payloads, namespaced by PID
            let dir = ipc_temp_dir();
            tokio::fs::create_dir_all(&dir).await.ok();
            let token = format!(
                "{}-{}-{}",
                std::process::id(),
                self.request_id.load(Ordering::Relaxed),
                chrono::Utc::now().timestamp_millis()
            );
            let file_path = dir.join(format!("{}.json", token));
            tokio::fs::write(&file_path, &serialized)
                .await
                .map_err(|e| PluginSubprocessError::Protocol(format!("ipc write: {}", e)))?;

            // Set restrictive permissions (Unix only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o600);
                std::fs::set_permissions(&file_path, perms).ok();
            }

            let file_params = serde_json::json!({
                "file": file_path.to_string_lossy(),
                "token": token,
            });
            let result = self
                .call::<Value>("hook.invoke.file", Some(file_params))
                .await;

            // Always cleanup temp file, even on error
            tokio::fs::remove_file(&file_path).await.ok();

            let result = result?;
            Ok(result.get("output").cloned().unwrap_or(Value::Null))
        } else {
            let result: Value = self.call("hook.invoke", Some(params)).await?;
            Ok(result.get("output").cloned().unwrap_or(Value::Null))
        }
    }

    /// Trigger OAuth authorization flow.
    pub async fn auth_authorize(
        &self,
        method_index: usize,
        inputs: Option<Value>,
    ) -> Result<AuthorizeResult, PluginSubprocessError> {
        let params = serde_json::json!({
            "methodIndex": method_index,
            "inputs": inputs.unwrap_or(Value::Null),
        });
        self.call("auth.authorize", Some(params)).await
    }

    /// Complete OAuth callback.
    pub async fn auth_callback(&self, code: Option<&str>) -> Result<Value, PluginSubprocessError> {
        let params = serde_json::json!({ "code": code });
        self.call("auth.callback", Some(params)).await
    }

    /// Load auth provider configuration.
    pub async fn auth_load(&self, provider: &str) -> Result<AuthLoadResult, PluginSubprocessError> {
        let params = serde_json::json!({ "provider": provider });
        self.call("auth.load", Some(params)).await
    }

    /// Proxy an HTTP request through the plugin's custom fetch.
    pub async fn auth_fetch(
        &self,
        url: &str,
        method: &str,
        headers: &std::collections::HashMap<String, String>,
        body: Option<&str>,
    ) -> Result<AuthFetchResult, PluginSubprocessError> {
        let params = serde_json::json!({
            "url": url,
            "method": method,
            "headers": headers,
            "body": body,
        });
        self.call("auth.fetch", Some(params)).await
    }

    /// Proxy an HTTP request through the plugin's custom fetch as a real-time stream.
    pub async fn auth_fetch_stream(
        &self,
        url: &str,
        method: &str,
        headers: &std::collections::HashMap<String, String>,
        body: Option<&str>,
    ) -> Result<AuthFetchStreamResult, PluginSubprocessError> {
        let id = self.next_id();
        let params = serde_json::json!({
            "url": url,
            "method": method,
            "headers": headers,
            "body": body,
        });

        let rpc_guard = self.rpc_lock.clone().lock_owned().await;
        self.write_request_with_timeout(id, "auth.fetch.stream", Some(params))
            .await?;

        let (start_tx, start_rx) = oneshot::channel::<
            Result<(u16, std::collections::HashMap<String, String>), PluginSubprocessError>,
        >();
        let (chunk_tx, chunk_rx) = mpsc::channel(128);
        let transport = Arc::clone(&self.transport);
        let plugin_name = self.name.clone();

        tokio::spawn(async move {
            let _rpc_guard = rpc_guard;
            let mut start_tx = Some(start_tx);
            let mut transport_guard = transport.write().await;
            let reader = &mut transport_guard.stdout;

            loop {
                let raw = match Self::read_raw_message(reader).await {
                    Ok(raw) => raw,
                    Err(err) => {
                        if let Some(tx) = start_tx.take() {
                            let _ = tx.send(Err(err));
                        } else {
                            if let Err(send_error) = chunk_tx.send(Err(err)).await {
                                tracing::debug!(
                                    plugin = %plugin_name,
                                    error = %send_error,
                                    "Failed to forward plugin stream startup error to consumer"
                                );
                            }
                        }
                        break;
                    }
                };

                if raw.get("id").and_then(Value::as_u64) == Some(id) {
                    let response: RpcResponse = match serde_json::from_value(raw) {
                        Ok(response) => response,
                        Err(err) => {
                            let send_err = PluginSubprocessError::Json(err);
                            if let Some(tx) = start_tx.take() {
                                let _ = tx.send(Err(send_err));
                            } else {
                                if let Err(error) = chunk_tx.send(Err(send_err)).await {
                                    tracing::debug!(
                                        plugin = %plugin_name,
                                        error = %error,
                                        "Failed to forward plugin stream JSON decode error to consumer"
                                    );
                                }
                            }
                            break;
                        }
                    };

                    if let Some(error) = response.error {
                        let send_err = PluginSubprocessError::from(error);
                        if let Some(tx) = start_tx.take() {
                            let _ = tx.send(Err(send_err));
                        } else {
                            if let Err(error) = chunk_tx.send(Err(send_err)).await {
                                tracing::debug!(
                                    plugin = %plugin_name,
                                    error = %error,
                                    "Failed to forward plugin stream RPC error to consumer"
                                );
                            }
                        }
                        break;
                    }

                    let result = response.result.unwrap_or(Value::Null);
                    let status = result
                        .get("status")
                        .and_then(Value::as_u64)
                        .and_then(|v| u16::try_from(v).ok())
                        .unwrap_or(200);
                    let headers = result
                        .get("headers")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    if let Some(tx) = start_tx.take() {
                        let _ = tx.send(Ok((status, headers)));
                    }
                    continue;
                }

                let method = raw.get("method").and_then(Value::as_str);
                let params = raw.get("params").cloned().unwrap_or(Value::Null);
                let request_id = params.get("requestId").and_then(Value::as_u64);
                if request_id != Some(id) {
                    continue;
                }

                match method {
                    Some("auth.fetch.stream.chunk") => {
                        if let Some(chunk) = params.get("chunk").and_then(Value::as_str) {
                            if chunk_tx.send(Ok(chunk.to_string())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some("auth.fetch.stream.error") => {
                        let message = params
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("plugin custom fetch stream failed")
                            .to_string();
                        let error = PluginSubprocessError::Protocol(message);
                        if let Some(tx) = start_tx.take() {
                            let _ = tx.send(Err(error));
                        } else {
                            if let Err(send_error) = chunk_tx.send(Err(error)).await {
                                tracing::debug!(
                                    plugin = %plugin_name,
                                    error = %send_error,
                                    "Failed to forward plugin stream protocol error to consumer"
                                );
                            }
                        }
                        break;
                    }
                    Some("auth.fetch.stream.end") => {
                        break;
                    }
                    _ => {}
                }
            }
        });

        let (status, response_headers) = tokio::time::timeout(self.timeout, start_rx)
            .await
            .map_err(|_| PluginSubprocessError::Timeout)?
            .map_err(|_| PluginSubprocessError::NotRunning)??;

        Ok(AuthFetchStreamResult {
            status,
            headers: response_headers,
            chunks: chunk_rx,
        })
    }

    /// Gracefully shut down the plugin subprocess.
    pub async fn shutdown(&self) -> Result<(), PluginSubprocessError> {
        // Guard drop auto-unregisters from ProcessRegistry (RAII).
        let _: Value = self.call("shutdown", None).await?;
        // Give the process a moment to exit, then kill if needed
        let mut transport = self.transport.write().await;
        match tokio::time::timeout(Duration::from_secs(2), transport.process.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::debug!(
                    plugin = %self.name,
                    error = %error,
                    "Failed while waiting for plugin subprocess shutdown"
                );
            }
            Err(error) => {
                tracing::debug!(
                    plugin = %self.name,
                    error = %error,
                    "Timed out waiting for plugin subprocess shutdown"
                );
            }
        }
        if let Err(error) = transport.process.kill().await {
            tracing::debug!(
                plugin = %self.name,
                error = %error,
                "Failed to kill plugin subprocess during shutdown"
            );
        }
        Ok(())
    }

    /// Send a cancel notification to abort a running request.
    pub async fn cancel_request(&self, request_id: u64) -> Result<(), PluginSubprocessError> {
        self.write_notification(
            "$/cancelRequest",
            Some(serde_json::json!({ "id": request_id })),
        )
        .await
    }

    // -- Self-heal (in-place reconnect) --------------------------------------

    /// Kill the current subprocess and spawn a fresh one, swapping the inner
    /// transport without replacing the outer `Arc<PluginSubprocess>`.
    ///
    /// SAFETY: Must only be called while `rpc_lock` is held (i.e. from within
    /// `call()`), so no concurrent RPC can observe a half-swapped transport.
    async fn reconnect(&self) -> Result<(), PluginSubprocessError> {
        tracing::warn!(plugin = %self.name, "[plugin-heal] reconnecting after timeout");

        // 1. Defuse old guard and kill old process
        {
            // Defuse the existing guard so it won't try to unregister the old PID
            if let Some(ref guard) = *self._process_guard.lock().unwrap() {
                guard.defuse();
            }
            let mut transport = self.transport.write().await;
            if let Err(error) = transport.process.kill().await {
                tracing::debug!(
                    plugin = %self.name,
                    error = %error,
                    "[plugin-heal] failed to kill old subprocess before reconnect"
                );
            }
        }

        // 2. Spawn new process
        let args = self.runtime.run_args(&self.host_script);
        let mut cmd = Command::new(self.runtime.command());
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = &self.cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn()?;

        // Drain stderr in background
        if let Some(stderr) = child.stderr.take() {
            let _handle = spawn_stderr_drain(
                stderr,
                StderrDrainConfig::new(format!("plugin:{}", self.plugin_path)),
            );
        }

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginSubprocessError::Protocol("no stdin on reconnect".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginSubprocessError::Protocol("no stdout on reconnect".into()))?;

        // 3. Swap transport
        {
            let mut transport = self.transport.write().await;
            *transport = Transport {
                stdin,
                stdout: BufReader::new(stdout),
                process: child,
            };
        }

        // 4. Re-initialize — write/read directly (rpc_lock already held by caller)
        let params = serde_json::json!({
            "pluginPath": &self.plugin_path,
            "context": &self.init_context,
        });
        let id = self.next_id();
        self.write_request(id, "initialize", Some(params)).await?;
        let response = tokio::time::timeout(self.timeout, self.read_response_for_id(id))
            .await
            .map_err(|_| PluginSubprocessError::Timeout)??;

        if let Some(err) = response.error {
            return Err(err.into());
        }

        // 5. Register new PID
        {
            let transport = self.transport.read().await;
            if let Some(pid) = transport.process.id() {
                let guard = global_registry().register_with_shutdown(
                    pid,
                    self.name.clone(),
                    ProcessKind::Plugin,
                    Arc::new({
                        let name = self.name.clone();
                        move || {
                            tracing::debug!(plugin = %name, "Plugin on_shutdown callback fired");
                        }
                    }),
                );
                *self._process_guard.lock().unwrap() = Some(guard);
            }
        }

        tracing::info!(plugin = %self.name, "[plugin-heal] reconnected successfully");
        Ok(())
    }

    // -- Transport (Content-Length framing) ----------------------------------

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a JSON-RPC request and wait for the response.
    ///
    /// On timeout, triggers an in-place reconnect so subsequent calls work.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T, PluginSubprocessError> {
        let _rpc_guard = self.rpc_lock.lock().await;
        let id = self.next_id();
        self.write_request_with_timeout(id, method, params).await?;

        // Use deadline that can be reset on progress notification
        let mut deadline = tokio::time::Instant::now() + self.timeout;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    // Timeout — attempt reconnect so the *next* call works.
                    if crate::feature_flags::is_enabled("plugin_timeout_self_heal") {
                        if let Err(error) = self.reconnect().await {
                            tracing::debug!(
                                plugin = %self.name,
                                error = %error,
                                "[plugin-heal] reconnect failed after RPC timeout"
                            );
                        }
                    }
                    return Err(PluginSubprocessError::Timeout);
                }
                message = self.read_message() => {
                    match message? {
                        super::protocol::RpcMessage::Response(resp) if resp.id == id => {
                            if let Some(err) = resp.error {
                                return Err(err.into());
                            }
                            let result = resp.result.unwrap_or(Value::Null);
                            return serde_json::from_value(result).map_err(Into::into);
                        }
                        ref msg if msg.is_progress_notification() => {
                            // Reset deadline on progress notification
                            deadline = tokio::time::Instant::now() + self.timeout;
                            tracing::debug!(
                                plugin = %self.name,
                                "progress notification received, deadline reset"
                            );
                        }
                        _ => continue,
                    }
                }
            }
        }
    }

    async fn write_request(
        &self,
        id: u64,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), PluginSubprocessError> {
        let request = RpcRequest::new(id, method, params);
        let mut transport = self.transport.write().await;
        codec::write_frame(&mut transport.stdin, &request).await?;
        Ok(())
    }

    async fn write_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), PluginSubprocessError> {
        let notification = super::protocol::RpcNotification::new(method, params);
        let mut transport = self.transport.write().await;
        codec::write_frame(&mut transport.stdin, &notification).await?;
        Ok(())
    }

    async fn write_request_with_timeout(
        &self,
        id: u64,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), PluginSubprocessError> {
        tokio::time::timeout(self.timeout, self.write_request(id, method, params))
            .await
            .map_err(|_| PluginSubprocessError::Timeout)?
    }

    async fn read_response_for_id(
        &self,
        expected_id: u64,
    ) -> Result<RpcResponse, PluginSubprocessError> {
        let mut transport = self.transport.write().await;
        let reader = &mut transport.stdout;
        loop {
            let raw = Self::read_raw_message(reader).await?;
            if raw.get("id").and_then(Value::as_u64) != Some(expected_id) {
                continue;
            }
            let response: RpcResponse = serde_json::from_value(raw)?;
            return Ok(response);
        }
    }

    async fn read_message(&self) -> Result<super::protocol::RpcMessage, PluginSubprocessError> {
        let mut transport = self.transport.write().await;
        let reader = &mut transport.stdout;
        let raw = Self::read_raw_message(reader).await?;
        super::protocol::RpcMessage::from_value(raw).map_err(Into::into)
    }

    /// Read one Content-Length framed JSON-RPC message from stdout.
    async fn read_raw_message(
        reader: &mut BufReader<ChildStdout>,
    ) -> Result<Value, PluginSubprocessError> {
        codec::read_frame(reader).await.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_temp_dir_is_pid_scoped() {
        let dir = ipc_temp_dir();
        let pid = std::process::id().to_string();
        assert!(
            dir.ends_with(&pid),
            "ipc_temp_dir should end with current PID, got {:?}",
            dir
        );
        assert!(
            dir.to_string_lossy().contains("agendao-plugin-ipc"),
            "ipc_temp_dir should contain agendao-plugin-ipc"
        );
    }
}
