use async_trait::async_trait;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use agendao_core::codec;
use agendao_core::process_registry::{global_registry, ProcessGuard, ProcessKind};
use agendao_core::stderr_drain::{spawn_stderr_drain, StderrDrainConfig};

use crate::protocol::{JsonRpcMessage, JsonRpcRequest};
use crate::McpClientError;

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

pub struct StdioTransport {
    process: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    /// Persistent buffered reader — avoids the BufReader-per-call data loss bug.
    stdout: Mutex<Option<BufReader<ChildStdout>>>,
    /// RAII guard — auto-unregisters from ProcessRegistry on drop.
    _process_guard: Option<ProcessGuard>,
}

impl StdioTransport {
    pub async fn new(
        command: &str,
        args: &[String],
        env: Option<Vec<(String, String)>>,
    ) -> Result<Self, McpClientError> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                cmd.env(key, value);
            }
        }
        let mut child = cmd.spawn().map_err(|e| {
            McpClientError::TransportError(format!("Failed to spawn process: {}", e))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpClientError::TransportError("Failed to get stdin".to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpClientError::TransportError("Failed to get stdout".to_string()))?;

        // Drain stderr so the pipe buffer doesn't deadlock the child.
        if let Some(stderr) = child.stderr.take() {
            let label = format!("mcp:{}", command);
            let _handle = spawn_stderr_drain(stderr, StderrDrainConfig::new(label));
        }

        // Register child with global ProcessRegistry for visibility.
        let child_pid = child.id().unwrap_or(0);
        let process_guard = if child_pid > 0 {
            Some(global_registry().register(
                child_pid,
                format!("mcp:{}", command),
                ProcessKind::Mcp,
            ))
        } else {
            None
        };

        Ok(Self {
            process: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            stdout: Mutex::new(Some(BufReader::new(stdout))),
            _process_guard: process_guard,
        })
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
        let mut process_guard = self.process.lock().await;
        if let Some(mut child) = process_guard.take() {
            child.kill().await.map_err(|e| {
                McpClientError::TransportError(format!("Failed to kill process: {}", e))
            })?;
        }
        let mut stdin_guard = self.stdin.lock().await;
        *stdin_guard = None;

        // Guard auto-unregisters from ProcessRegistry when StdioTransport is dropped (RAII).

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

        let resp =
            builder.body(body).send().await.map_err(|e| {
                McpClientError::TransportError(format!("HTTP request failed: {}", e))
            })?;

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

        let resp = builder
            .body(body)
            .send()
            .await
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
