use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use lsp_types::{
    ClientCapabilities, Diagnostic, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    InitializeParams, Range, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentItem, VersionedTextDocumentIdentifier, WorkspaceFolder,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use agendao_core::codec;
use agendao_core::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use agendao_core::process_registry::{global_registry, ProcessKind};
use agendao_core::stderr_drain::{spawn_stderr_drain, StderrDrainConfig};
use agendao_sandbox::{
    IntegrationSandboxContext, PrepareOptions, SandboxHandleDriver, SpawnSpec, StdioPlan, StdioSpec,
};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{debug, error};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: lsp_types::SymbolKind,
    #[serde(default)]
    pub tags: Option<Vec<lsp_types::SymbolTag>>,
    #[serde(default)]
    pub detail: Option<String>,
    pub uri: lsp_types::Uri,
    pub range: Range,
    pub selection_range: Range,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyPrepareParams {
    #[serde(flatten)]
    pub text_document_position_params: lsp_types::TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done_progress_params: lsp_types::WorkDoneProgressParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyIncomingCallsParams {
    pub item: CallHierarchyItem,
    #[serde(flatten)]
    pub work_done_progress_params: lsp_types::WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result_params: lsp_types::PartialResultParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyOutgoingCallsParams {
    pub item: CallHierarchyItem,
    #[serde(flatten)]
    pub work_done_progress_params: lsp_types::WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result_params: lsp_types::PartialResultParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyIncomingCall {
    pub from: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyOutgoingCall {
    pub to: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

fn path_to_uri(path: &Path) -> Result<lsp_types::Uri, LspError> {
    let url = Url::from_file_path(path)
        .map_err(|_| LspError::InitializeError("Invalid file path".to_string()))?;
    lsp_types::Uri::from_str(url.as_str())
        .map_err(|e| LspError::InitializeError(format!("Invalid URI: {}", e)))
}

fn uri_to_path(uri: &lsp_types::Uri) -> Option<PathBuf> {
    let raw = uri.to_string();
    match Url::parse(&raw).ok().and_then(|u| u.to_file_path().ok()) {
        Some(path) => Some(path),
        None => {
            tracing::warn!(uri = %raw, "failed to convert LSP URI to file path");
            None
        }
    }
}

#[derive(Debug, Error)]
pub enum LspError {
    #[error("Failed to start LSP server: {0}")]
    ServerStartError(String),

    #[error("Failed to initialize LSP: {0}")]
    InitializeError(String),

    #[error("JSON-RPC error: {0}")]
    JsonRpcError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Server not initialized")]
    NotInitialized,

    #[error("Timeout waiting for response")]
    Timeout,
}

impl From<codec::CodecError> for LspError {
    fn from(e: codec::CodecError) -> Self {
        match e {
            codec::CodecError::Io(io) => Self::IoError(io),
            codec::CodecError::Serialize(se) => Self::SerializationError(se),
            codec::CodecError::Protocol(msg) => Self::JsonRpcError(msg),
            codec::CodecError::ConnectionClosed => Self::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed",
            )),
        }
    }
}

pub struct LspServerConfig {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub initialization_options: Option<Value>,
}

type PendingResponseTx = tokio::sync::oneshot::Sender<Result<Value, LspError>>;
type PendingResponses = Arc<RwLock<HashMap<u64, PendingResponseTx>>>;

/// Timeout for a single LSP request. Language servers can hang; without a
/// bound a `request()` would wait forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for event-driven diagnostics waits.
///
/// Replaces the previous fixed 100–200 ms sleeps: servers usually publish
/// diagnostics within a few hundred milliseconds, but cold starts can take
/// longer. Reaching the bound is not an error — callers proceed with
/// whatever diagnostics are available.
pub const DIAGNOSTICS_WAIT_TIMEOUT: Duration = Duration::from_secs(2);

pub struct LspClient {
    root: PathBuf,
    stdin: Arc<Mutex<ChildStdin>>,
    /// The sandbox driver: sole owner of the execution handle, selecting
    /// between the child's natural exit and the TERM → grace → KILL
    /// ladder. Replaces both the raw `Child` and its `kill_on_drop`.
    driver: Arc<Mutex<Option<SandboxHandleDriver>>>,
    request_id: Arc<Mutex<u64>>,
    pending_responses: PendingResponses,
    diagnostics: Arc<RwLock<HashMap<PathBuf, Vec<Diagnostic>>>>,
    file_versions: Arc<RwLock<HashMap<PathBuf, u32>>>,
    event_tx: broadcast::Sender<LspEvent>,
}

#[derive(Debug, Clone)]
pub enum LspEvent {
    Diagnostics { path: PathBuf, server_id: String },
}

impl LspClient {
    /// Start a language server. The process is launched through the
    /// sandbox execution boundary as `TrustClass::UserConfiguredIntegration`
    /// under the `Integration` profile (contained, workspace-scoped,
    /// network denied) — there is no direct-spawn path and no
    /// "already sandboxed" escape hatch a server config could widen
    /// (sandbox plan Phase 6).
    pub async fn start(
        config: LspServerConfig,
        root: PathBuf,
        sandbox: IntegrationSandboxContext,
    ) -> Result<Self, LspError> {
        let spec = SpawnSpec {
            program: config.command.clone(),
            args: config.args.clone(),
            cwd: Some(root.clone()),
            env_overrides: Default::default(),
        };
        // The request fixes the trust class and profile kind; user config
        // only picks the binary and args — it cannot widen them.
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
                LspError::ServerStartError(format!("sandbox denied the LSP server launch: {}", e))
            })?;
        let mut handle = prepared.start().await.map_err(|e| {
            LspError::ServerStartError(format!("LSP server process spawn failed: {}", e))
        })?;

        let child_pid = handle.pid().unwrap_or(0);

        let stdin = handle
            .take_stdin()
            .ok_or_else(|| LspError::ServerStartError("Failed to get stdin".to_string()))?;
        let stdout = handle
            .take_stdout()
            .ok_or_else(|| LspError::ServerStartError("Failed to get stdout".to_string()))?;

        // Drain stderr to prevent pipe-buffer deadlock.
        if let Some(stderr) = handle.take_stderr() {
            let _handle = spawn_stderr_drain(
                stderr,
                StderrDrainConfig::new(format!("lsp:{}", config.command)),
            );
        }

        let (event_tx, _) = broadcast::channel(256);

        // The registry guard and the driver reference each other (the
        // guard's shutdown hook terminates through the driver; the
        // driver's exit callback drops the guard, unregistering the pid
        // the moment the process dies), so the hook binds to a late slot
        // filled synchronously right after spawn. Same shape as the MCP
        // stdio transport and `shell_session`.
        let late_driver: Arc<std::sync::OnceLock<SandboxHandleDriver>> =
            Arc::new(std::sync::OnceLock::new());
        let hook_driver = late_driver.clone();
        let process_guard = if child_pid > 0 {
            Some(global_registry().register_with_shutdown(
                child_pid,
                format!("lsp:{}", config.command),
                ProcessKind::Lsp,
                Arc::new(move || {
                    if let Some(driver) = hook_driver.get() {
                        let driver = driver.clone();
                        let _ = tokio::spawn(async move { driver.terminate().await });
                    }
                }),
            ))
        } else {
            None
        };

        let exit_command = config.command.clone();
        let driver = SandboxHandleDriver::spawn(handle, move |status| {
            debug!(command = exit_command, ?status, "LSP server process exited");
            // Drop the guard on exit so the registry stops listing a
            // dead pid before the client itself goes away.
            drop(process_guard);
        });
        let _ = late_driver.set(driver.clone());

        let client = Self {
            root,
            stdin: Arc::new(Mutex::new(stdin)),
            driver: Arc::new(Mutex::new(Some(driver))),
            request_id: Arc::new(Mutex::new(0)),
            pending_responses: Arc::new(RwLock::new(HashMap::new())),
            diagnostics: Arc::new(RwLock::new(HashMap::new())),
            file_versions: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        };

        let pending = client.pending_responses.clone();
        let diagnostics = client.diagnostics.clone();
        let server_id = config.id.clone();
        let event_tx_clone = client.event_tx.clone();

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if line.is_empty() || line.starts_with("Content-Length:") {
                    continue;
                }

                if let Ok(notification) = serde_json::from_str::<JsonRpcNotification>(&line) {
                    if notification.method == "textDocument/publishDiagnostics" {
                        if let Some(params) = notification.params {
                            if let Ok(diag_params) = serde_json::from_value::<
                                lsp_types::PublishDiagnosticsParams,
                            >(params)
                            {
                                let Some(path) = uri_to_path(&diag_params.uri) else {
                                    continue;
                                };

                                diagnostics
                                    .write()
                                    .await
                                    .insert(path.clone(), diag_params.diagnostics);

                                let _ = event_tx_clone.send(LspEvent::Diagnostics {
                                    path,
                                    server_id: server_id.clone(),
                                });
                            }
                        }
                    }
                } else if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&line) {
                    if let Some(sender) = pending.write().await.remove(&response.id) {
                        let result = if let Some(error) = response.error {
                            Err(LspError::JsonRpcError(error.message))
                        } else {
                            Ok(response.result.unwrap_or(Value::Null))
                        };
                        let _ = sender.send(result);
                    }
                }
            }
        });

        let mut client = client;
        client.initialize(config.initialization_options).await?;

        Ok(client)
    }

    async fn initialize(&mut self, initialization_options: Option<Value>) -> Result<(), LspError> {
        let workspace_uri = path_to_uri(&self.root)?;

        let params = InitializeParams {
            initialization_options,
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: workspace_uri,
                name: "workspace".to_string(),
            }]),
            capabilities: ClientCapabilities::default(),
            ..Default::default()
        };

        let result = self
            .request("initialize", serde_json::to_value(params)?)
            .await?;
        debug!(?result, "LSP initialized");

        self.notify("initialized", Value::Null).await?;

        Ok(())
    }

    async fn next_id(&self) -> u64 {
        let mut id = self.request_id.lock().await;
        *id += 1;
        *id
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, LspError> {
        let id = self.next_id().await;
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.pending_responses.write().await.insert(id, tx);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params: Some(params),
        };

        // Hold the stdin lock only for the write, not while waiting.
        let write_result = {
            let mut stdin = self.stdin.lock().await;
            codec::write_frame(&mut *stdin, &request).await
        };
        if let Err(e) = write_result {
            self.pending_responses.write().await.remove(&id);
            return Err(e.into());
        }

        let result = match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            // Sender dropped (reader task ended) or deadline elapsed.
            Ok(Err(_)) | Err(_) => Err(LspError::Timeout),
        };

        // Remove the pending entry so a late response is dropped instead of
        // leaking in the map.
        self.pending_responses.write().await.remove(&id);

        result
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params) },
        };

        let mut stdin = self.stdin.lock().await;
        codec::write_frame(&mut *stdin, &notification).await?;

        Ok(())
    }

    pub async fn open_document(
        &self,
        path: &Path,
        content: &str,
        language_id: &str,
    ) -> Result<(), LspError> {
        let uri = path_to_uri(path)?;

        let version = {
            let mut versions = self.file_versions.write().await;
            let v = versions.entry(path.to_path_buf()).or_insert(0);
            *v
        };

        if version > 0 {
            let next_version = version + 1;
            self.file_versions
                .write()
                .await
                .insert(path.to_path_buf(), next_version);

            let params = DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri,
                    version: next_version as i32,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: content.to_string(),
                }],
            };

            self.notify("textDocument/didChange", serde_json::to_value(params)?)
                .await?;
        } else {
            self.file_versions
                .write()
                .await
                .insert(path.to_path_buf(), 0);
            self.diagnostics.write().await.remove(path);

            let params = DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri,
                    language_id: language_id.to_string(),
                    version: 0,
                    text: content.to_string(),
                },
            };

            self.notify("textDocument/didOpen", serde_json::to_value(params)?)
                .await?;
        }

        Ok(())
    }

    pub async fn get_diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        self.diagnostics
            .read()
            .await
            .get(path)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns all diagnostics from all files this LSP server has reported on.
    pub async fn get_all_diagnostics(&self) -> HashMap<PathBuf, Vec<Diagnostic>> {
        self.diagnostics.read().await.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LspEvent> {
        self.event_tx.subscribe()
    }

    /// Wait until a `publishDiagnostics` notification for `path` arrives, or
    /// `timeout` elapses. Returns `true` if the event was observed.
    ///
    /// Best-effort: reaching the timeout is not an error, callers proceed
    /// with whatever diagnostics are available.
    pub async fn wait_diagnostics_for(&self, path: &Path, timeout: Duration) -> bool {
        let mut rx = self.subscribe();
        let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        tokio::time::timeout(timeout, async {
            loop {
                match rx.recv().await {
                    Ok(LspEvent::Diagnostics {
                        path: event_path, ..
                    }) => {
                        let event_path = event_path
                            .canonicalize()
                            .unwrap_or_else(|_| event_path.clone());
                        if event_path == target {
                            return true;
                        }
                    }
                    // Keep waiting through lagged events.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return false,
                }
            }
        })
        .await
        .unwrap_or(false)
    }

    pub async fn goto_definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Location>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("textDocument/definition", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let response: lsp_types::GotoDefinitionResponse = serde_json::from_value(result)?;
        match response {
            lsp_types::GotoDefinitionResponse::Scalar(loc) => Ok(Some(loc)),
            lsp_types::GotoDefinitionResponse::Array(locs) => Ok(locs.into_iter().next()),
            lsp_types::GotoDefinitionResponse::Link(links) => {
                Ok(links.into_iter().next().map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_selection_range,
                }))
            }
        }
    }

    pub async fn completion(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<Vec<lsp_types::CompletionItem>>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::CompletionParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        };

        let result = self
            .request("textDocument/completion", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let response: lsp_types::CompletionResponse = serde_json::from_value(result)?;
        Ok(Some(match response {
            lsp_types::CompletionResponse::Array(items) => items,
            lsp_types::CompletionResponse::List(list) => list.items,
        }))
    }

    pub async fn references(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::ReferenceParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
        };

        let result = self
            .request("textDocument/references", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let locations: Vec<lsp_types::Location> = serde_json::from_value(result)?;
        Ok(locations)
    }

    pub async fn hover(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Hover>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = self
            .request("textDocument/hover", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let hover: lsp_types::Hover = serde_json::from_value(result)?;
        Ok(Some(hover))
    }

    pub async fn document_symbol(
        &self,
        path: &Path,
    ) -> Result<Vec<lsp_types::SymbolInformation>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("textDocument/documentSymbol", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let symbols: lsp_types::DocumentSymbolResponse = serde_json::from_value(result)?;
        Ok(match symbols {
            lsp_types::DocumentSymbolResponse::Flat(symbols) => symbols,
            lsp_types::DocumentSymbolResponse::Nested(nested) => nested
                .into_iter()
                .flat_map(|s| flatten_document_symbol(&s))
                .collect(),
        })
    }

    pub async fn workspace_symbol(
        &self,
        query: &str,
    ) -> Result<Vec<lsp_types::SymbolInformation>, LspError> {
        let params = lsp_types::WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("workspace/symbol", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let symbols: Vec<lsp_types::SymbolInformation> = serde_json::from_value(result)?;
        Ok(symbols)
    }

    pub async fn goto_implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::request::GotoImplementationParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("textDocument/implementation", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let response: lsp_types::GotoDefinitionResponse = serde_json::from_value(result)?;
        Ok(match response {
            lsp_types::GotoDefinitionResponse::Scalar(loc) => vec![loc],
            lsp_types::GotoDefinitionResponse::Array(locs) => locs,
            lsp_types::GotoDefinitionResponse::Link(links) => links
                .into_iter()
                .map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_selection_range,
                })
                .collect(),
        })
    }

    pub async fn type_definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: lsp_types::Position { line, character },
        };

        let result = self
            .request("textDocument/typeDefinition", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let response: lsp_types::GotoDefinitionResponse = serde_json::from_value(result)?;
        Ok(match response {
            lsp_types::GotoDefinitionResponse::Scalar(loc) => vec![loc],
            lsp_types::GotoDefinitionResponse::Array(locs) => locs,
            lsp_types::GotoDefinitionResponse::Link(links) => links
                .into_iter()
                .map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_selection_range,
                })
                .collect(),
        })
    }

    pub async fn rename(
        &self,
        path: &Path,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<Option<lsp_types::WorkspaceEdit>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::RenameParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        };

        let result = self
            .request("textDocument/rename", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let edit: lsp_types::WorkspaceEdit = serde_json::from_value(result)?;
        Ok(Some(edit))
    }

    pub async fn prepare_call_hierarchy(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<CallHierarchyItem>, LspError> {
        let uri = path_to_uri(path)?;

        let params = CallHierarchyPrepareParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = self
            .request(
                "textDocument/prepareCallHierarchy",
                serde_json::to_value(params)?,
            )
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let items: Vec<CallHierarchyItem> = serde_json::from_value(result)?;
        Ok(items)
    }

    pub async fn incoming_calls(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<CallHierarchyIncomingCall>, LspError> {
        let items = self.prepare_call_hierarchy(path, line, character).await?;

        if items.is_empty() {
            return Ok(vec![]);
        }

        let item = &items[0];
        let params = CallHierarchyIncomingCallsParams {
            item: item.clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("callHierarchy/incomingCalls", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let calls: Vec<CallHierarchyIncomingCall> = serde_json::from_value(result)?;
        Ok(calls)
    }

    pub async fn outgoing_calls(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<CallHierarchyOutgoingCall>, LspError> {
        let items = self.prepare_call_hierarchy(path, line, character).await?;

        if items.is_empty() {
            return Ok(vec![]);
        }

        let item = &items[0];
        let params = CallHierarchyOutgoingCallsParams {
            item: item.clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("callHierarchy/outgoingCalls", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let calls: Vec<CallHierarchyOutgoingCall> = serde_json::from_value(result)?;
        Ok(calls)
    }

    /// Graceful shutdown: sends LSP `shutdown` + `exit`, then runs the
    /// cancellation ladder (TERM → grace → KILL) through the sandbox
    /// driver. The process guard is dropped by the driver's exit
    /// callback, unregistering the pid.
    pub async fn shutdown(&self) {
        // Try LSP protocol shutdown
        if let Err(error) = self.request("shutdown", serde_json::Value::Null).await {
            tracing::debug!(
                error = %error,
                "Failed to send LSP shutdown request during client shutdown"
            );
        }
        if let Err(error) = self.notify("exit", serde_json::Value::Null).await {
            tracing::debug!(
                error = %error,
                "Failed to send LSP exit notification during client shutdown"
            );
        }

        let mut driver_guard = self.driver.lock().await;
        if let Some(driver) = driver_guard.take() {
            if let Err(error) = driver.terminate().await {
                tracing::debug!(
                    error = %error,
                    "Failed to terminate LSP child process during shutdown"
                );
            }
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // kill_on_drop equivalent: the driver task owns the handle, so a
        // dropped client must terminate the ladder explicitly — a
        // language server must never outlive its client just because the
        // caller forgot `shutdown()`. Drop cannot await, so the ladder
        // runs detached; `shutdown()` remains the synchronous path.
        if let Ok(mut guard) = self.driver.try_lock() {
            if let Some(driver) = guard.take() {
                let _ = tokio::spawn(async move { driver.terminate().await });
            }
        }
    }
}

#[allow(deprecated)]
fn flatten_document_symbol(
    symbol: &lsp_types::DocumentSymbol,
) -> Vec<lsp_types::SymbolInformation> {
    let mut result = vec![];

    result.push(lsp_types::SymbolInformation {
        name: symbol.name.clone(),
        kind: symbol.kind,
        tags: symbol.tags.clone(),
        deprecated: symbol.deprecated,
        location: lsp_types::Location {
            uri: path_to_uri(&std::path::PathBuf::new())
                .unwrap_or_else(|_| lsp_types::Uri::from_str("file:///").unwrap()),
            range: symbol.selection_range,
        },
        container_name: symbol.detail.clone(),
    });

    if let Some(children) = &symbol.children {
        for child in children {
            result.extend(flatten_document_symbol(child));
        }
    }

    result
}

pub struct LspClientRegistry {
    clients: RwLock<HashMap<String, Arc<LspClient>>>,
}

impl LspClientRegistry {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, id: String, client: Arc<LspClient>) {
        self.clients.write().await.insert(id, client);
    }

    /// Shut down and remove an LSP client by ID.
    pub async fn remove(&self, id: &str) {
        let client = self.clients.write().await.remove(id);
        if let Some(client) = client {
            client.shutdown().await;
        }
    }

    pub async fn get(&self, id: &str) -> Option<Arc<LspClient>> {
        self.clients.read().await.get(id).cloned()
    }

    pub async fn list(&self) -> Vec<(String, Arc<LspClient>)> {
        self.clients
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Returns true if any LSP clients are currently registered.
    /// Mirrors the TS `LSP.hasClients(file)` which checks whether any server
    /// could handle the given file. Here we check if any registered client's
    /// id contains the detected language for the file.
    pub async fn has_clients(&self, path: &Path) -> bool {
        let clients = self.clients.read().await;
        if clients.is_empty() {
            return false;
        }
        let language = detect_language(path);
        clients.keys().any(|id| id.contains(language))
    }

    /// Opens or refreshes a file in all matching LSP clients.
    /// Mirrors the TS `LSP.touchFile(input, waitForDiagnostics)`.
    ///
    /// - Reads the file content from disk
    /// - For each registered client whose id matches the file's language,
    ///   calls `open_document` (which internally handles didOpen vs didChange)
    /// - If `wait_for_diagnostics` is true, waits briefly for diagnostics to arrive
    pub async fn touch_file(
        &self,
        path: &Path,
        wait_for_diagnostics: bool,
    ) -> Result<(), LspError> {
        let language = detect_language(path);
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(LspError::IoError)?;

        let clients = self.clients.read().await;
        let matching: Vec<Arc<LspClient>> = clients
            .iter()
            .filter(|(id, _)| id.contains(language))
            .map(|(_, c)| c.clone())
            .collect();
        drop(clients);

        for client in &matching {
            if let Err(e) = client.open_document(path, &content, language).await {
                error!("Failed to touch file {:?} in LSP: {}", path, e);
            }
        }

        if wait_for_diagnostics && !matching.is_empty() {
            // Event-driven: wait for this file's publishDiagnostics instead
            // of a fixed sleep. Reaching the bound is not an error.
            for client in &matching {
                client
                    .wait_diagnostics_for(path, DIAGNOSTICS_WAIT_TIMEOUT)
                    .await;
            }
        }

        Ok(())
    }
}

impl Default for LspClientRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn detect_language(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("js") => "javascript",
        Some("jsx") => "javascriptreact",
        Some("py") => "python",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") => "c",
        Some("cpp") | Some("cc") | Some("cxx") => "cpp",
        Some("h") | Some("hpp") => "cpp",
        Some("rb") => "ruby",
        Some("php") => "php",
        Some("swift") => "swift",
        Some("kt") => "kotlin",
        Some("scala") => "scala",
        Some("lua") => "lua",
        Some("json") => "json",
        Some("yaml") | Some("yml") => "yaml",
        Some("toml") => "toml",
        Some("md") => "markdown",
        Some("html") => "html",
        Some("css") => "css",
        Some("scss") => "scss",
        _ => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language(Path::new("main.rs")), "rust");
        assert_eq!(detect_language(Path::new("index.ts")), "typescript");
        assert_eq!(detect_language(Path::new("app.tsx")), "typescriptreact");
        assert_eq!(detect_language(Path::new("main.py")), "python");
        assert_eq!(detect_language(Path::new("main.go")), "go");
        assert_eq!(detect_language(Path::new("unknown.xyz")), "plaintext");
    }

    #[test]
    fn test_json_rpc_request_serialization() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "initialize".to_string(),
            params: Some(Value::Null),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"initialize\""));
    }

    #[test]
    fn test_json_rpc_response_deserialization() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#;
        let response: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.id, 1);
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_has_clients_empty_registry() {
        let registry = LspClientRegistry::new();
        assert!(!registry.has_clients(Path::new("main.rs")).await);
        assert!(!registry.has_clients(Path::new("index.ts")).await);
    }

    #[tokio::test]
    async fn test_registry_default() {
        let registry = LspClientRegistry::default();
        let clients = registry.list().await;
        assert!(clients.is_empty());
    }

    #[tokio::test]
    async fn test_registry_remove_nonexistent() {
        let registry = LspClientRegistry::new();
        // Removing a non-existent ID should not panic
        registry.remove("nonexistent").await;
        assert!(registry.list().await.is_empty());
    }
}
