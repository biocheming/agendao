//! Reusable stderr drain for child processes.
//!
//! Spawns a background task that reads stderr and logs via `tracing`,
//! with configurable rate-limiting to prevent log flooding.
//! This is the single authority for stderr handling — all subprocess
//! clients (Plugin, MCP, LSP) should use this instead of rolling their own.

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStderr;
use tokio::task::JoinHandle;

/// Configuration for stderr drain behavior.
pub struct StderrDrainConfig {
    /// Label used in tracing fields (e.g. "plugin:my-plugin", "mcp:sqlite").
    pub label: String,
    /// Maximum number of lines to log per rate window. Lines beyond this
    /// are silently dropped. Set to `0` for unlimited.
    pub max_lines_per_window: u64,
    /// Duration of the rate-limiting window.
    pub window_duration: Duration,
}

impl StderrDrainConfig {
    /// Default config: 20 lines per second, suitable for most subprocesses.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            max_lines_per_window: 20,
            window_duration: Duration::from_secs(1),
        }
    }

    /// Unlimited logging (no rate limit). Use for short-lived processes
    /// like bash commands where all output is valuable.
    pub fn unlimited(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            max_lines_per_window: 0,
            window_duration: Duration::from_secs(1),
        }
    }
}

/// Spawn a background task that drains stderr and logs each line.
///
/// Returns the [`JoinHandle`] so the caller can optionally await or cancel it.
/// The task runs until stderr EOF or read error.
///
/// # Example
/// ```ignore
/// let handle = spawn_stderr_drain(stderr, StderrDrainConfig::new("mcp:sqlite"));
/// // ... later, on shutdown:
/// handle.abort();
/// ```
pub fn spawn_stderr_drain(stderr: ChildStderr, config: StderrDrainConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        let mut count = 0u64;
        let mut last_reset = tokio::time::Instant::now();
        let unlimited = config.max_lines_per_window == 0;

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let msg = line.trim_end();
                    if msg.is_empty() {
                        continue;
                    }

                    if unlimited {
                        tracing::warn!(
                            subprocess = %config.label,
                            "[stderr] {}", msg
                        );
                        continue;
                    }

                    // Rate limiting
                    if last_reset.elapsed() > config.window_duration {
                        if count > config.max_lines_per_window {
                            let dropped = count - config.max_lines_per_window;
                            tracing::debug!(
                                subprocess = %config.label,
                                "stderr rate limit: dropped {dropped} lines in last window"
                            );
                        }
                        count = 0;
                        last_reset = tokio::time::Instant::now();
                    }
                    count += 1;
                    if count <= config.max_lines_per_window {
                        tracing::warn!(
                            subprocess = %config.label,
                            "[stderr] {}", msg
                        );
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        subprocess = %config.label,
                        %error,
                        "stderr drain ended"
                    );
                    break;
                }
            }
        }
    })
}
