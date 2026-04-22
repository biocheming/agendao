//! Server discovery, startup, and health-check utilities for CLI.
//!
//! CLI shares the same HTTP server as TUI and Web. On startup the CLI
//! probes the configured address; if a server is already running it reuses
//! it, otherwise it starts an in-process backend runtime.

use std::time::Duration;

use crate::util::server_url;
use rocode_launcher::wait_for_server_ready;
use tokio::process::Command;

/// Default port when nothing else is configured.
const DEFAULT_PORT: u16 = 3000;

/// Maximum time to wait for a freshly spawned server to become ready.
const SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

// ── public API ───────────────────────────────────────────────────────

/// Resolve the base URL from environment variables, falling back to the
/// default port.
pub(crate) fn resolve_server_url(port_override: Option<u16>) -> String {
    // 1. Explicit URL override
    if let Ok(url) =
        std::env::var("ROCODE_SERVER_URL").or_else(|_| std::env::var("OPENCODE_SERVER_URL"))
    {
        let url = url.trim().to_string();
        if !url.is_empty() {
            return url;
        }
    }

    // 2. TUI base-url (set when launched from `rocode --tui`)
    if let Ok(url) =
        std::env::var("ROCODE_TUI_BASE_URL").or_else(|_| std::env::var("OPENCODE_TUI_BASE_URL"))
    {
        let url = url.trim().to_string();
        if !url.is_empty() {
            return url;
        }
    }

    // 3. Build from port
    let port = port_override.unwrap_or(DEFAULT_PORT);
    format!("http://127.0.0.1:{}", port)
}

/// Discover a running server or start a new one.
///
/// Returns the base URL of the server that is ready to accept requests.
pub(crate) async fn discover_or_start_server(port_override: Option<u16>) -> anyhow::Result<String> {
    let base_url = resolve_server_url(port_override);

    // 1. Probe existing server
    if health_check(&base_url).await.is_ok() {
        tracing::info!("Connected to existing server at {}", base_url);
        return Ok(base_url);
    }

    // 2. Start a new local product-host process in server mode
    let port = port_override.unwrap_or(DEFAULT_PORT);
    tracing::info!(
        "No server found — starting local server on 127.0.0.1:{}",
        port
    );
    let current_exe = std::env::current_exe()?;
    let mut child = Command::new(current_exe)
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .arg("--hostname")
        .arg("127.0.0.1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    // 3. Wait until the server is ready
    wait_for_server_ready(&base_url, SERVER_STARTUP_TIMEOUT, Some(&mut child)).await?;
    tokio::spawn(async move {
        match child.wait().await {
            Ok(status) => tracing::warn!("Background rocode host exited with status {}", status),
            Err(error) => tracing::warn!("Background rocode host wait failed: {}", error),
        }
    });

    tracing::info!("Local server ready at {}", base_url);
    Ok(base_url)
}

/// Quick health check — returns `Ok(())` if the server responds 2xx on
/// `/health`.
pub(crate) async fn health_check(base_url: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let url = server_url(base_url, "/health");
    let resp = client.get(&url).send().await?;
    if resp.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("Health check failed: {}", resp.status());
    }
}
