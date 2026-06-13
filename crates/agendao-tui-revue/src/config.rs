//! 土 — Application configuration authority.
//!
//! Old TUI: AppLaunchConfig with env var overrides + 3 transport modes.
//! New: AppConfig matches the same pattern.

use std::path::PathBuf;

/// Application launch configuration.
/// Transport priority: local_direct > unix_socket > base_url (HTTP SSE).
#[derive(Clone, Debug)]
pub struct AppConfig {
    // ── Transport ──
    /// Run in-process (default: true). No server/HTTP needed.
    pub local_direct: bool,
    /// Unix socket path for local IPC transport.
    pub unix_socket_path: Option<String>,
    /// HTTP base URL for SSE event stream (only when !local_direct).
    pub base_url: Option<String>,

    // ── Session ──
    pub agent_name: Option<String>,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub initial_prompt: Option<String>,
    pub working_dir: Option<PathBuf>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            local_direct: env_bool("AGENDAO_TUI_LOCAL_DIRECT").unwrap_or(true),
            unix_socket_path: env_str("AGENDAO_UNIX_SOCKET"),
            base_url: env_str("AGENDAO_TUI_BASE_URL"),
            agent_name: env_str("AGENDAO_TUI_AGENT"),
            model: env_str("AGENDAO_TUI_MODEL"),
            session_id: env_str("AGENDAO_TUI_SESSION"),
            initial_prompt: env_str("AGENDAO_TUI_PROMPT"),
            working_dir: env_str("AGENDAO_TUI_DIR").map(PathBuf::from),
        }
    }
}

fn env_str(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn env_bool(key: &str) -> Option<bool> {
    env_str(key).map(|v| matches!(v.as_str(), "1" | "true"))
}

/// Initialize logging to file (mirrors old agendao::init_logging).
pub fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let log_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("agendao").join("log");
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(log_dir.join("agendao.log")).ok();
    if let Some(file) = log_file {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("warn"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false).init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("warn")).init();
    }
}
