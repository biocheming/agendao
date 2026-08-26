use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::BufReader;
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use agendao_core::codec;
use agendao_core::process_registry::{global_registry, ProcessKind};
use agendao_core::stderr_drain::{spawn_stderr_drain, StderrDrainConfig};
use agendao_sandbox::{
    IntegrationSandboxContext, PrepareOptions, SandboxHandleDriver, SpawnSpec, StdioPlan, StdioSpec,
};

use crate::McpClientError;
use agendao_core::jsonrpc::{JsonRpcMessage, JsonRpcRequest};

/// Bound on the HTTP send/response-header phase of MCP POSTs.
///
/// reqwest applies no total timeout by default, so a hung server could block
/// a POST forever. Response bodies stay unbounded: long tool calls stream
/// progress frames and are governed by the client's own request timeout.
const HTTP_SEND_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Transport trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn send(&self, request: &JsonRpcRequest) -> Result<(), McpClientError>;
    async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError>;
    async fn close(&self) -> Result<(), McpClientError>;
}

// ---------------------------------------------------------------------------
// StdioTransport
// ---------------------------------------------------------------------------

/// stdio transport for a user-configured MCP server process.
///
/// The child is launched through the sandbox execution boundary as
/// `TrustClass::UserConfiguredIntegration` under the `Integration`
/// profile (contained, workspace-scoped, network denied) — no direct
/// spawn path remains here, and no "already sandboxed" escape hatch a
/// server config could widen (sandbox plan Phase 6).
pub struct StdioTransport {
    stdin: Mutex<Option<ChildStdin>>,
    /// Persistent buffered reader — avoids the BufReader-per-call data loss bug.
    stdout: Mutex<Option<BufReader<ChildStdout>>>,
    /// The sandbox driver: sole owner of the execution handle, selecting
    /// between the child's natural exit and the TERM → grace → KILL
    /// ladder. Replaces both the raw `Child` and its `kill_on_drop`.
    driver: Mutex<Option<SandboxHandleDriver>>,
}

impl StdioTransport {
    pub async fn new(
        command: &str,
        args: &[String],
        env: Option<Vec<(String, String)>>,
        sandbox: IntegrationSandboxContext,
    ) -> Result<Self, McpClientError> {
        let spec = SpawnSpec {
            program: command.to_string(),
            args: args.to_vec(),
            cwd: Some(sandbox.workspace.clone()),
            env_overrides: env.unwrap_or_default().into_iter().collect(),
        };
        // The request fixes the trust class and profile kind; user config
        // only picks the binary, args, cwd and env — it cannot widen them.
        let prepared = sandbox
            .prepare(
                spec,
                PrepareOptions {
                    stdio: StdioPlan {
                        stdin: StdioSpec::Piped,
                        stdout: StdioSpec::Piped,
                        stderr: StdioSpec::Piped,
                    },
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| {
                McpClientError::TransportError(format!(
                    "sandbox denied the MCP server launch: {}",
                    e
                ))
            })?;
        let mut handle = prepared.start().await.map_err(|e| {
            McpClientError::TransportError(format!("MCP server process spawn failed: {}", e))
        })?;

        let stdin = handle
            .take_stdin()
            .ok_or_else(|| McpClientError::TransportError("Failed to get stdin".to_string()))?;

        let stdout = handle
            .take_stdout()
            .ok_or_else(|| McpClientError::TransportError("Failed to get stdout".to_string()))?;

        // Drain stderr so the pipe buffer doesn't deadlock the child.
        if let Some(stderr) = handle.take_stderr() {
            let label = format!("mcp:{}", command);
            let _handle = spawn_stderr_drain(stderr, StderrDrainConfig::new(label));
        }

        // The registry guard and the driver reference each other (the
        // guard's shutdown hook terminates through the driver; the
        // driver's exit callback drops the guard, unregistering the pid
        // the moment the process dies), so the hook binds to a late slot
        // filled synchronously right after spawn — before any shutdown
        // could fire. Same shape as `shell_session`.
        let child_pid = handle.pid().unwrap_or(0);
        let late_driver: std::sync::Arc<std::sync::OnceLock<SandboxHandleDriver>> =
            std::sync::Arc::new(std::sync::OnceLock::new());
        let hook_driver = late_driver.clone();
        let process_guard = if child_pid > 0 {
            Some(global_registry().register_with_shutdown(
                child_pid,
                format!("mcp:{}", command),
                ProcessKind::Mcp,
                std::sync::Arc::new(move || {
                    if let Some(driver) = hook_driver.get() {
                        let driver = driver.clone();
                        drop(tokio::spawn(async move { driver.terminate().await }));
                    }
                }),
            ))
        } else {
            None
        };

        let exit_command = command.to_string();
        let driver = SandboxHandleDriver::spawn(handle, move |status| {
            tracing::debug!(command = exit_command, ?status, "MCP server process exited");
            // Drop the guard on exit so the registry stops listing a
            // dead pid before the transport itself goes away.
            drop(process_guard);
        });
        let _ = late_driver.set(driver.clone());

        Ok(Self {
            stdin: Mutex::new(Some(stdin)),
            stdout: Mutex::new(Some(BufReader::new(stdout))),
            driver: Mutex::new(Some(driver)),
        })
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // kill_on_drop equivalent: the driver task owns the handle, so a
        // dropped transport must terminate the ladder explicitly — an
        // MCP server must never outlive its client just because the
        // caller forgot `close()`. Drop cannot await, so the ladder runs
        // detached; `close()` remains the synchronous, error-reporting
        // path.
        if let Ok(mut guard) = self.driver.try_lock() {
            if let Some(driver) = guard.take() {
                drop(tokio::spawn(async move { driver.terminate().await }));
            }
        }
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<(), McpClientError> {
        let mut stdin_guard = self.stdin.lock().await;
        let stdin = stdin_guard
            .as_mut()
            .ok_or_else(|| McpClientError::TransportError("Process not running".to_string()))?;

        codec::write_frame(stdin, request)
            .await
            .map_err(|e| McpClientError::TransportError(format!("Failed to write: {}", e)))?;

        Ok(())
    }

    async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError> {
        let mut stdout_guard = self.stdout.lock().await;
        let reader = stdout_guard
            .as_mut()
            .ok_or_else(|| McpClientError::TransportError("Process not running".to_string()))?;

        match codec::read_frame(reader).await {
            Ok(value) => {
                let message = JsonRpcMessage::from_value(value).map_err(|e| {
                    McpClientError::ProtocolError(format!("Failed to parse message: {}", e))
                })?;
                Ok(Some(message))
            }
            Err(codec::CodecError::ConnectionClosed) => Ok(None),
            Err(e) => Err(McpClientError::TransportError(format!(
                "Failed to read: {}",
                e
            ))),
        }
    }

    async fn close(&self) -> Result<(), McpClientError> {
        // Drop stdout reader first to release the pipe.
        {
            let mut stdout_guard = self.stdout.lock().await;
            *stdout_guard = None;
        }
        // The cancellation ladder (TERM → grace → KILL), not a bare
        // kill: an MCP server gets the chance to shut down cleanly.
        {
            let mut driver_guard = self.driver.lock().await;
            if let Some(driver) = driver_guard.take() {
                driver.terminate().await.map_err(|e| {
                    McpClientError::TransportError(format!("Failed to terminate process: {}", e))
                })?;
            }
        }
        let mut stdin_guard = self.stdin.lock().await;
        *stdin_guard = None;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HttpTransport (StreamableHTTP)
// ---------------------------------------------------------------------------

/// Transport that sends JSON-RPC requests over HTTP POST and reads streaming
/// (potentially chunked) JSON responses. Mirrors the TS `StreamableHTTPClientTransport`.
pub struct HttpTransport {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
    /// Buffer for responses received via streaming that haven't been consumed yet.
    response_rx: Mutex<tokio::sync::mpsc::UnboundedReceiver<JsonRpcMessage>>,
    response_tx: tokio::sync::mpsc::UnboundedSender<JsonRpcMessage>,
}

impl HttpTransport {
    pub fn new(url: String, headers: Option<HashMap<String, String>>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            url,
            headers: headers.unwrap_or_default(),
            client: reqwest::Client::new(),
            response_rx: Mutex::new(rx),
            response_tx: tx,
        }
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<(), McpClientError> {
        let mut builder = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        for (key, value) in &self.headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        let body = serde_json::to_string(request).map_err(|e| {
            McpClientError::ProtocolError(format!("Failed to serialize request: {}", e))
        })?;

        let resp = tokio::time::timeout(HTTP_SEND_TIMEOUT, builder.body(body).send())
            .await
            .map_err(|_| McpClientError::Timeout)?
            .map_err(|e| McpClientError::TransportError(format!("HTTP request failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(McpClientError::TransportError(format!(
                "HTTP {} from server",
                resp.status()
            )));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // Server chose to stream the response via SSE inside the POST response.
            let text = resp.text().await.map_err(|e| {
                McpClientError::TransportError(format!("Failed to read SSE body: {}", e))
            })?;
            for line in text.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    match data.parse::<JsonRpcMessage>() {
                        Ok(message) => {
                            if self.response_tx.send(message).is_err() {
                                tracing::warn!(
                                    "HttpTransport: response channel closed, dropping SSE message"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("HttpTransport: failed to parse SSE message: {}", e);
                        }
                    }
                }
            }
        } else {
            // Plain JSON response.
            let text = resp.text().await.map_err(|e| {
                McpClientError::TransportError(format!("Failed to read response body: {}", e))
            })?;
            if !text.is_empty() {
                let message = text.parse::<JsonRpcMessage>().map_err(|e| {
                    McpClientError::ProtocolError(format!("Failed to parse response: {}", e))
                })?;
                self.response_tx.send(message).map_err(|_| {
                    McpClientError::TransportError("HttpTransport: response channel closed".into())
                })?;
            }
        }

        Ok(())
    }

    async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError> {
        let mut rx = self.response_rx.lock().await;
        match rx.recv().await {
            Some(msg) => Ok(Some(msg)),
            None => Ok(None),
        }
    }

    async fn close(&self) -> Result<(), McpClientError> {
        // Nothing to tear down – the reqwest client will be dropped with the struct.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SseTransport
// ---------------------------------------------------------------------------

/// Transport that connects to an SSE endpoint for receiving messages and
/// POSTs JSON-RPC requests to the same base URL. Mirrors the TS
/// `SSEClientTransport`.
pub struct SseTransport {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
    response_rx: Mutex<tokio::sync::mpsc::UnboundedReceiver<JsonRpcMessage>>,
    response_tx: tokio::sync::mpsc::UnboundedSender<JsonRpcMessage>,
    /// Handle to the background SSE listener task so we can abort on close.
    sse_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SseTransport {
    pub fn new(url: String, headers: Option<HashMap<String, String>>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            url,
            headers: headers.unwrap_or_default(),
            client: reqwest::Client::new(),
            response_rx: Mutex::new(rx),
            response_tx: tx,
            sse_task: Mutex::new(None),
        }
    }

    /// Start the background SSE listener. Must be called before `send`/`receive`.
    pub async fn connect(&self) -> Result<(), McpClientError> {
        use futures::StreamExt;
        use reqwest_eventsource::{Event, EventSource};

        let mut builder = self.client.get(&self.url);
        builder = builder.header("Accept", "text/event-stream");
        for (key, value) in &self.headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        let mut es = EventSource::new(builder).map_err(|e| {
            McpClientError::TransportError(format!("Failed to create SSE connection: {}", e))
        })?;

        let tx = self.response_tx.clone();

        let handle = tokio::spawn(async move {
            loop {
                let Some(event) = StreamExt::next(&mut es).await else {
                    break;
                };
                match event {
                    Ok(Event::Message(msg)) => {
                        let data = msg.data.trim().to_string();
                        if data.is_empty() || data == "[DONE]" {
                            continue;
                        }
                        match data.parse::<JsonRpcMessage>() {
                            Ok(msg) => {
                                if tx.send(msg).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!("SSE: failed to parse message: {}", e);
                            }
                        }
                    }
                    Ok(Event::Open) => {
                        tracing::debug!("SSE connection opened");
                    }
                    Err(e) => {
                        tracing::error!("SSE error: {}", e);
                        break;
                    }
                }
            }
        });

        let mut task = self.sse_task.lock().await;
        *task = Some(handle);

        Ok(())
    }
}

#[async_trait]
impl McpTransport for SseTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<(), McpClientError> {
        let mut builder = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json");

        for (key, value) in &self.headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        let body = serde_json::to_string(request).map_err(|e| {
            McpClientError::ProtocolError(format!("Failed to serialize request: {}", e))
        })?;

        let resp = tokio::time::timeout(HTTP_SEND_TIMEOUT, builder.body(body).send())
            .await
            .map_err(|_| McpClientError::Timeout)?
            .map_err(|e| McpClientError::TransportError(format!("HTTP POST failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(McpClientError::TransportError(format!(
                "HTTP {} from server",
                resp.status()
            )));
        }

        Ok(())
    }

    async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError> {
        let mut rx = self.response_rx.lock().await;
        match rx.recv().await {
            Some(msg) => Ok(Some(msg)),
            None => Ok(None),
        }
    }

    async fn close(&self) -> Result<(), McpClientError> {
        let mut task = self.sse_task.lock().await;
        if let Some(handle) = task.take() {
            handle.abort();
        }
        Ok(())
    }
}
