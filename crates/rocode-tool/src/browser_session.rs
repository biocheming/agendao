use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use url::Url;

use crate::web_page::{
    convert_html_to_markdown, ensure_http_url, ensure_safe_http_url, extract_title, strip_html,
    DEFAULT_WEB_TIMEOUT_SECS, DEFAULT_WEB_USER_AGENT, MAX_WEB_RESPONSE_SIZE, MAX_WEB_TIMEOUT_SECS,
};
use crate::{Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

const DESCRIPTION: &str = r#"Structured browser-like HTTP session with cookies and current-page state.

Phase 1 operations:
- start: create a cookie-backed browser session
- visit: navigate to a URL or relative path using the session
- read: read the current page in markdown, text, or html form
- status: inspect current session/page state
- terminate: delete the session

This is not a JS browser. It is the single authority for stateful web page navigation in ROCode."#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BrowserSessionOperation {
    Start,
    Visit,
    Read,
    Status,
    Terminate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserSessionInput {
    operation: BrowserSessionOperation,
    #[serde(default, alias = "sessionId", alias = "session_id")]
    session_id: Option<String>,
    #[serde(default, alias = "base_url")]
    base_url: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default, alias = "user_agent")]
    user_agent: Option<String>,
    #[serde(default = "default_format")]
    format: String,
    #[serde(default)]
    timeout: Option<u64>,
}

fn default_format() -> String {
    "markdown".to_string()
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionView {
    id: String,
    base_url: Option<String>,
    current_url: Option<String>,
    current_title: Option<String>,
    current_status: Option<u16>,
    history_len: usize,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserLink {
    text: String,
    href: String,
    resolved_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserPageSnapshot {
    url: String,
    status: u16,
    content_type: String,
    title: Option<String>,
    html: String,
    links: Vec<BrowserLink>,
    fetched_at: i64,
}

struct BrowserSessionRecord {
    id: String,
    base_url: Option<Url>,
    client: Client,
    headers: HashMap<String, String>,
    created_at: i64,
    state: Arc<RwLock<BrowserSessionState>>,
}

#[derive(Default)]
struct BrowserSessionState {
    current_page: Option<BrowserPageSnapshot>,
    history: Vec<String>,
    cookies: HashMap<String, String>,
}

struct BrowserSessionManager {
    sessions: Arc<RwLock<HashMap<String, Arc<BrowserSessionRecord>>>>,
}

impl BrowserSessionManager {
    fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn create_session(
        &self,
        input: &BrowserSessionInput,
    ) -> Result<BrowserSessionView, ToolError> {
        let id = format!("browser_{}", uuid::Uuid::new_v4().simple());
        let base_url = if let Some(base_url) = input
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(ensure_safe_http_url(base_url).await?)
        } else {
            None
        };

        let user_agent = input
            .user_agent
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_WEB_USER_AGENT);
        let timeout_secs = input
            .timeout
            .unwrap_or(DEFAULT_WEB_TIMEOUT_SECS)
            .min(MAX_WEB_TIMEOUT_SECS);

        let client = Client::builder()
            .user_agent(user_agent)
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| {
                ToolError::ExecutionError(format!("failed to build browser session client: {}", e))
            })?;

        let record = Arc::new(BrowserSessionRecord {
            id: id.clone(),
            base_url,
            client,
            headers: input.headers.clone(),
            created_at: chrono::Utc::now().timestamp(),
            state: Arc::new(RwLock::new(BrowserSessionState::default())),
        });
        self.sessions.write().await.insert(id, record.clone());
        Ok(record.view().await)
    }

    async fn get(&self, session_id: &str) -> Result<Arc<BrowserSessionRecord>, ToolError> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| {
                ToolError::ExecutionError(format!("browser session `{}` was not found", session_id))
            })
    }

    async fn remove(&self, session_id: &str) -> Result<BrowserSessionView, ToolError> {
        let record = self
            .sessions
            .write()
            .await
            .remove(session_id)
            .ok_or_else(|| {
                ToolError::ExecutionError(format!("browser session `{}` was not found", session_id))
            })?;
        Ok(record.view().await)
    }
}

impl BrowserSessionRecord {
    async fn view(&self) -> BrowserSessionView {
        let state = self.state.read().await;
        BrowserSessionView {
            id: self.id.clone(),
            base_url: self.base_url.as_ref().map(|url| url.to_string()),
            current_url: state.current_page.as_ref().map(|page| page.url.clone()),
            current_title: state
                .current_page
                .as_ref()
                .and_then(|page| page.title.clone()),
            current_status: state.current_page.as_ref().map(|page| page.status),
            history_len: state.history.len(),
            created_at: self.created_at,
        }
    }
}

static BROWSER_SESSION_MANAGER: OnceLock<BrowserSessionManager> = OnceLock::new();

fn browser_session_manager() -> &'static BrowserSessionManager {
    BROWSER_SESSION_MANAGER.get_or_init(BrowserSessionManager::new)
}

pub struct BrowserSessionTool;

impl BrowserSessionTool {
    pub fn new() -> Self {
        Self
    }

    async fn execute_impl(
        &self,
        input: BrowserSessionInput,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        validate_input(&input)?;
        match input.operation {
            BrowserSessionOperation::Start => self.start(input).await,
            BrowserSessionOperation::Visit => self.visit(input, ctx).await,
            BrowserSessionOperation::Read => self.read(input).await,
            BrowserSessionOperation::Status => self.status(input).await,
            BrowserSessionOperation::Terminate => self.terminate(input).await,
        }
    }

    async fn start(&self, input: BrowserSessionInput) -> Result<ToolResult, ToolError> {
        let session = browser_session_manager().create_session(&input).await?;
        Ok(ToolResult {
            title: "Browser Session Started".to_string(),
            output: format!("Started browser session {}.", session.id),
            metadata: session_metadata("start", &session),
            truncated: false,
        })
    }

    async fn visit(
        &self,
        input: BrowserSessionInput,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = browser_session_manager().get(&session_id).await?;
        let target_url = resolve_target_url(&session, &input).await?;
        ctx.ask_permission(
            PermissionRequest::new("webfetch")
                .with_pattern(target_url.as_str())
                .with_metadata("url", serde_json::json!(target_url.as_str()))
                .always_allow(),
        )
        .await?;

        let snapshot = fetch_page(&session, target_url.clone(), &ctx).await?;
        {
            let mut state = session.state.write().await;
            state.history.push(snapshot.url.clone());
            state.current_page = Some(snapshot.clone());
        }
        let session_view = session.view().await;
        let output = render_visit_output(&snapshot);
        let mut metadata = session_metadata("visit", &session_view);
        metadata.insert("page".to_string(), serde_json::to_value(&snapshot).unwrap());
        Ok(ToolResult {
            title: format!("Browser Visit: {}", snapshot.url),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn read(&self, input: BrowserSessionInput) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = browser_session_manager().get(&session_id).await?;
        let session_view = session.view().await;
        let state = session.state.read().await;
        let page = state.current_page.as_ref().ok_or_else(|| {
            ToolError::ExecutionError("browser session has no current page".to_string())
        })?;
        let output = render_page(page, &input.format)?;
        let mut metadata = session_metadata("read", &session_view);
        metadata.insert("page".to_string(), serde_json::to_value(page).unwrap());
        metadata.insert("format".to_string(), serde_json::json!(input.format));
        Ok(ToolResult {
            title: format!("Browser Read: {}", page.url),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn status(&self, input: BrowserSessionInput) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = browser_session_manager().get(&session_id).await?;
        let session_view = session.view().await;
        Ok(ToolResult {
            title: "Browser Session Status".to_string(),
            output: format!(
                "Browser session {} current URL: {}",
                session_view.id,
                session_view.current_url.as_deref().unwrap_or("<none>")
            ),
            metadata: session_metadata("status", &session_view),
            truncated: false,
        })
    }

    async fn terminate(&self, input: BrowserSessionInput) -> Result<ToolResult, ToolError> {
        let session_id = required_session_id(&input)?;
        let session = browser_session_manager().remove(&session_id).await?;
        Ok(ToolResult {
            title: "Browser Session Terminated".to_string(),
            output: format!("Removed browser session {}.", session.id),
            metadata: session_metadata("terminate", &session),
            truncated: false,
        })
    }
}

impl Default for BrowserSessionTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserSessionTool {
    fn id(&self) -> &str {
        "browser_session"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["start", "visit", "read", "status", "terminate"]
                },
                "session_id": {
                    "type": "string",
                    "description": "Existing browser session id"
                },
                "base_url": {
                    "type": "string",
                    "description": "Optional absolute base URL for relative navigation"
                },
                "url": {
                    "type": "string",
                    "description": "Absolute or relative target URL for visit"
                },
                "path": {
                    "type": "string",
                    "description": "Alias for a relative or absolute target used by visit"
                },
                "headers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Session-wide request headers"
                },
                "user_agent": {
                    "type": "string",
                    "description": "Optional browser user agent for start"
                },
                "format": {
                    "type": "string",
                    "enum": ["markdown", "text", "html"],
                    "default": "markdown",
                    "description": "Output format for read"
                },
                "timeout": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional session request timeout in seconds"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: BrowserSessionInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        self.execute_impl(input, ctx).await
    }
}

fn validate_input(input: &BrowserSessionInput) -> Result<(), ToolError> {
    match input.operation {
        BrowserSessionOperation::Start => {
            if let Some(base_url) = input
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                ensure_http_url(base_url)?;
            }
        }
        BrowserSessionOperation::Visit => {
            required_session_id(input)?;
            let target = input
                .url
                .as_deref()
                .or(input.path.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ToolError::InvalidArguments("visit requires url or path".to_string())
                })?;
            if target.starts_with("http://") || target.starts_with("https://") {
                ensure_http_url(target)?;
            }
        }
        BrowserSessionOperation::Read
        | BrowserSessionOperation::Status
        | BrowserSessionOperation::Terminate => {
            required_session_id(input)?;
        }
    }

    match input.format.as_str() {
        "markdown" | "text" | "html" => Ok(()),
        _ => Err(ToolError::InvalidArguments(
            "format must be one of: markdown, text, html".to_string(),
        )),
    }
}

fn required_session_id(input: &BrowserSessionInput) -> Result<String, ToolError> {
    input
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .ok_or_else(|| {
            ToolError::InvalidArguments("session_id is required for this operation".to_string())
        })
}

async fn resolve_target_url(
    session: &BrowserSessionRecord,
    input: &BrowserSessionInput,
) -> Result<Url, ToolError> {
    let target = input
        .url
        .as_deref()
        .or(input.path.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments("visit requires url or path".to_string()))?;

    if target.starts_with("http://") || target.starts_with("https://") {
        return ensure_safe_http_url(target).await;
    }

    let state = session.state.read().await;
    let current = state
        .current_page
        .as_ref()
        .and_then(|page| Url::parse(&page.url).ok());
    if let Some(current) = current {
        let joined = current.join(target).map_err(|e| {
            ToolError::InvalidArguments(format!("invalid relative target `{}`: {}", target, e))
        })?;
        return ensure_safe_http_url(joined.as_str()).await;
    }
    drop(state);

    let base = session.base_url.as_ref().ok_or_else(|| {
        ToolError::InvalidArguments("relative visit requires current page or base_url".to_string())
    })?;
    let joined = base.join(target).map_err(|e| {
        ToolError::InvalidArguments(format!("invalid relative target `{}`: {}", target, e))
    })?;
    ensure_safe_http_url(joined.as_str()).await
}

async fn fetch_page(
    session: &BrowserSessionRecord,
    target_url: Url,
    ctx: &ToolContext,
) -> Result<BrowserPageSnapshot, ToolError> {
    let cookies = {
        let state = session.state.read().await;
        state.cookies.clone()
    };
    let mut request = session.client.get(target_url.clone());
    for (name, value) in &session.headers {
        request = request.header(name, value);
    }
    if !cookies.is_empty() {
        let cookie_header = cookies
            .iter()
            .map(|(name, value)| format!("{}={}", name, value))
            .collect::<Vec<_>>()
            .join("; ");
        request = request.header(reqwest::header::COOKIE, cookie_header);
    }
    let response = tokio::select! {
        result = request.send() => result,
        _ = ctx.abort.cancelled() => {
            return Err(ToolError::Cancelled);
        }
    }
    .map_err(|e| ToolError::ExecutionError(format!("browser session request failed: {}", e)))?;

    let status = response.status().as_u16();
    let set_cookie_values = response.headers().get_all(reqwest::header::SET_COOKIE);
    let new_cookies = parse_set_cookie_headers(
        set_cookie_values
            .iter()
            .filter_map(|value| value.to_str().ok()),
    );
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let body = response.text().await.map_err(|e| {
        ToolError::ExecutionError(format!("failed to read browser session response: {}", e))
    })?;
    if body.len() > MAX_WEB_RESPONSE_SIZE {
        return Err(ToolError::ExecutionError(
            "Response too large (exceeds 5MB limit)".to_string(),
        ));
    }
    let title = if content_type.contains("html") {
        extract_title(&body)
    } else {
        None
    };
    let links = if content_type.contains("html") {
        extract_links(&body, &target_url)
    } else {
        Vec::new()
    };

    if !new_cookies.is_empty() {
        let mut state = session.state.write().await;
        for (name, value) in new_cookies {
            state.cookies.insert(name, value);
        }
    }

    Ok(BrowserPageSnapshot {
        url: target_url.to_string(),
        status,
        content_type,
        title,
        html: body,
        links,
        fetched_at: chrono::Utc::now().timestamp(),
    })
}

fn render_visit_output(snapshot: &BrowserPageSnapshot) -> String {
    let mut lines = Vec::new();
    lines.push(format!("URL: {}", snapshot.url));
    lines.push(format!("Status: {}", snapshot.status));
    lines.push(format!("Content-Type: {}", snapshot.content_type));
    if let Some(title) = &snapshot.title {
        lines.push(format!("Title: {}", title));
    }
    if !snapshot.links.is_empty() {
        lines.push("Links:".to_string());
        for link in snapshot.links.iter().take(8) {
            let resolved = link.resolved_url.as_deref().unwrap_or(&link.href);
            let text = if link.text.is_empty() {
                "<no text>"
            } else {
                link.text.as_str()
            };
            lines.push(format!("- {} -> {}", text, resolved));
        }
        if snapshot.links.len() > 8 {
            lines.push(format!("- ... {} more links", snapshot.links.len() - 8));
        }
    }
    lines.join("\n")
}

fn render_page(snapshot: &BrowserPageSnapshot, format: &str) -> Result<String, ToolError> {
    match format {
        "html" => Ok(snapshot.html.clone()),
        "markdown" => {
            if snapshot.content_type.contains("html") {
                Ok(convert_html_to_markdown(&snapshot.html))
            } else {
                Ok(snapshot.html.clone())
            }
        }
        "text" => {
            if snapshot.content_type.contains("html") {
                Ok(strip_html(&snapshot.html))
            } else {
                Ok(snapshot.html.clone())
            }
        }
        _ => Err(ToolError::InvalidArguments(
            "format must be one of: markdown, text, html".to_string(),
        )),
    }
}

fn parse_set_cookie_headers<'a>(values: impl Iterator<Item = &'a str>) -> Vec<(String, String)> {
    let mut cookies = Vec::new();
    for value in values {
        let first = value.split(';').next().unwrap_or_default().trim();
        if first.is_empty() {
            continue;
        }
        let Some((name, raw_value)) = first.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        cookies.push((name.to_string(), raw_value.trim().to_string()));
    }
    cookies
}

fn session_metadata(operation: &str, session: &BrowserSessionView) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert("operation".to_string(), serde_json::json!(operation));
    metadata.insert(
        "session".to_string(),
        serde_json::to_value(session).unwrap(),
    );
    metadata
}

fn extract_links(html: &str, current_url: &Url) -> Vec<BrowserLink> {
    static LINK_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = LINK_RE.get_or_init(|| {
        Regex::new(r#"(?is)<a\b[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#)
            .expect("link regex should compile")
    });

    let mut links = Vec::new();
    for captures in re.captures_iter(html) {
        let href = captures
            .get(1)
            .map(|m| m.as_str().trim())
            .unwrap_or_default();
        if href.is_empty() {
            continue;
        }
        let text = captures
            .get(2)
            .map(|m| strip_html(m.as_str()))
            .unwrap_or_default();
        let resolved_url = current_url.join(href).ok().map(|url| url.to_string());
        links.push(BrowserLink {
            text,
            href: href.to_string(),
            resolved_url,
        });
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    fn should_skip_local_http_test(err: &std::io::Error) -> bool {
        matches!(
            err.kind(),
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::AddrNotAvailable
        )
    }
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex as AsyncMutex;

    #[test]
    fn schema_exposes_browser_session_operations() {
        let schema = BrowserSessionTool::new().parameters();
        let operations = schema["properties"]["operation"]["enum"]
            .as_array()
            .expect("operation enum");
        assert!(operations.iter().any(|value| value == "start"));
        assert!(operations.iter().any(|value| value == "visit"));
        assert!(operations.iter().any(|value| value == "read"));
        assert!(operations.iter().any(|value| value == "status"));
        assert!(operations.iter().any(|value| value == "terminate"));
    }

    #[test]
    fn browser_session_input_accepts_snake_case_fields() {
        let input: BrowserSessionInput = serde_json::from_value(serde_json::json!({
            "operation": "visit",
            "session_id": "browser_123",
            "base_url": "http://127.0.0.1:8080",
            "user_agent": "rocode-test"
        }))
        .expect("snake_case browser session input should deserialize");

        assert_eq!(input.session_id.as_deref(), Some("browser_123"));
        assert_eq!(input.base_url.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(input.user_agent.as_deref(), Some("rocode-test"));
    }

    #[tokio::test]
    async fn browser_session_persists_cookies_across_visits() {
        let (base_url, server_handle) = match spawn_test_server().await {
            Ok(value) => value,
            Err(err) if should_skip_local_http_test(&err) => {
                eprintln!(
                    "skipping local browser_session test in current environment: {}",
                    err
                );
                return;
            }
            Err(err) => panic!("spawn_test_server should succeed: {}", err),
        };
        let permission_log = Arc::new(AsyncMutex::new(Vec::<String>::new()));
        let permission_log_clone = permission_log.clone();
        let dir = tempdir().expect("tempdir");
        let ctx = ToolContext::new(
            "session-browser".into(),
            "message-browser".into(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_ask(move |req| {
            let permission_log_clone = permission_log_clone.clone();
            async move {
                permission_log_clone.lock().await.push(req.permission);
                Ok(())
            }
        });

        let start = BrowserSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "start",
                    "base_url": base_url
                }),
                ctx.clone(),
            )
            .await
            .expect("start should succeed");
        let session_id = start.metadata["session"]["id"]
            .as_str()
            .expect("session id")
            .to_string();

        BrowserSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "visit",
                    "session_id": session_id,
                    "path": "/set-cookie"
                }),
                ctx.clone(),
            )
            .await
            .expect("first visit should succeed");

        BrowserSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "visit",
                    "session_id": start.metadata["session"]["id"],
                    "path": "/check-cookie"
                }),
                ctx.clone(),
            )
            .await
            .expect("second visit should succeed");

        let read = BrowserSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "read",
                    "session_id": start.metadata["session"]["id"],
                    "format": "text"
                }),
                ctx.clone(),
            )
            .await
            .expect("read should succeed");
        assert!(
            read.output.contains("cookie=present"),
            "page output: {}",
            read.output
        );

        let status = BrowserSessionTool::new()
            .execute(
                serde_json::json!({
                    "operation": "status",
                    "session_id": start.metadata["session"]["id"]
                }),
                ctx,
            )
            .await
            .expect("status should succeed");
        assert_eq!(
            status.metadata["session"]["history_len"],
            serde_json::json!(2)
        );

        let permission_log = permission_log.lock().await;
        assert_eq!(
            permission_log
                .iter()
                .filter(|item| item.as_str() == "webfetch")
                .count(),
            2
        );

        server_handle.abort();
    }

    async fn spawn_test_server() -> Result<(String, tokio::task::JoinHandle<()>), std::io::Error> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(value) => value,
                    Err(_) => break,
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let mut read = 0usize;
                    loop {
                        match socket.read(&mut buf[read..]).await {
                            Ok(0) => return,
                            Ok(n) => {
                                read += n;
                                if read >= 4
                                    && buf[..read].windows(4).any(|window| window == b"\r\n\r\n")
                                {
                                    break;
                                }
                                if read == buf.len() {
                                    break;
                                }
                            }
                            Err(_) => return,
                        }
                    }
                    let request = String::from_utf8_lossy(&buf[..read]).to_string();
                    let mut lines = request.lines();
                    let first = lines.next().unwrap_or_default();
                    let path = first.split_whitespace().nth(1).unwrap_or("/");
                    let cookie_present = request.contains("rocode_session=alpha");
                    let response = match path {
                        "/set-cookie" => http_response(
                            200,
                            &["Content-Type: text/html; charset=utf-8", "Set-Cookie: rocode_session=alpha; Path=/"],
                            "<html><head><title>Cookie Set</title></head><body><a href=\"/check-cookie\">Check</a><p>cookie set</p></body></html>",
                        ),
                        "/check-cookie" if cookie_present => http_response(
                            200,
                            &["Content-Type: text/html; charset=utf-8"],
                            "<html><head><title>Cookie Check</title></head><body><p>cookie=present</p></body></html>",
                        ),
                        "/check-cookie" => http_response(
                            200,
                            &["Content-Type: text/html; charset=utf-8"],
                            "<html><head><title>Cookie Check</title></head><body><p>cookie=missing</p></body></html>",
                        ),
                        _ => http_response(404, &["Content-Type: text/plain; charset=utf-8"], "not found"),
                    };
                    if let Err(error) = socket.write_all(response.as_bytes()).await {
                        tracing::debug!(
                            error = %error,
                            "Failed to write browser session local HTTP response"
                        );
                    }
                    if let Err(error) = socket.shutdown().await {
                        tracing::debug!(
                            error = %error,
                            "Failed to shutdown browser session local HTTP socket"
                        );
                    }
                });
            }
        });
        Ok((format!("http://{}", addr), handle))
    }

    fn http_response(status: u16, headers: &[&str], body: &str) -> String {
        let status_line = match status {
            200 => "HTTP/1.1 200 OK",
            404 => "HTTP/1.1 404 Not Found",
            _ => "HTTP/1.1 500 Internal Server Error",
        };
        let mut response = vec![status_line.to_string()];
        response.push(format!("Content-Length: {}", body.len()));
        response.extend(headers.iter().map(|value| (*value).to_string()));
        response.push("Connection: close".to_string());
        response.push(String::new());
        response.push(body.to_string());
        response.join("\r\n")
    }
}
