/// Transport selector - choose Unix socket explicitly or fall back to HTTP.

use super::FrontendTransport;
use anyhow::Result;
use std::path::Path;

/// Transport selection options
#[derive(Debug, Clone)]
pub struct TransportSelector {
    /// Unix socket path to try first
    pub unix_socket_path: Option<String>,
    /// HTTP base URL (fallback)
    pub http_base_url: String,
    /// HTTP server password
    pub http_password: Option<String>,
}

impl TransportSelector {
    /// Create a new transport selector
    pub fn new(
        unix_socket_path: Option<String>,
        http_base_url: String,
        http_password: Option<String>,
    ) -> Self {
        Self {
            unix_socket_path,
            http_base_url,
            http_password,
        }
    }

    /// Select the best available transport.
    ///
    /// Tries Unix Socket first, falls back to HTTP if unavailable.
    pub async fn select(&self) -> Result<FrontendTransport> {
        // Try Unix Socket first if path is provided
        if let Some(socket_path) = &self.unix_socket_path {
            if Path::new(socket_path).exists() {
                eprintln!("Attempting Unix Socket connection: {}", socket_path);

                // Try to connect to Unix Socket
                let transport = FrontendTransport::unix(socket_path.clone());

                // Test connection with a simple list_sessions call
                match transport.list_sessions().await {
                    Ok(_) => {
                        eprintln!("Unix Socket connection successful");
                        return Ok(transport);
                    }
                    Err(e) => {
                        eprintln!("Unix Socket connection failed: {}, falling back to HTTP", e);
                    }
                }
            } else {
                eprintln!("Unix Socket path does not exist: {}, using HTTP", socket_path);
            }
        }

        // Fallback to HTTP
        eprintln!("Using HTTP transport: {}", self.http_base_url);
        Ok(FrontendTransport::http(
            self.http_base_url.clone(),
            self.http_password.clone(),
        ))
    }

    /// Require a Unix socket transport.
    ///
    /// This is for explicit user intent such as `--socket`, where silently
    /// falling back to HTTP would violate the selected transport mode.
    pub async fn select_unix_required(&self) -> Result<FrontendTransport> {
        let Some(socket_path) = &self.unix_socket_path else {
            anyhow::bail!("Unix socket mode requested but no socket path was provided");
        };
        if !Path::new(socket_path).exists() {
            anyhow::bail!("Unix socket path does not exist: {}", socket_path);
        }

        eprintln!("Attempting Unix Socket connection: {}", socket_path);
        let transport = FrontendTransport::unix(socket_path.clone());
        transport
            .list_sessions()
            .await
            .map_err(|error| anyhow::anyhow!("Unix Socket connection failed: {}", error))?;
        eprintln!("Unix Socket connection successful");
        Ok(transport)
    }

    /// Get the default Unix socket path for the current platform
    pub fn default_unix_socket_path() -> Option<String> {
        #[cfg(unix)]
        {
            let candidates = vec![
                "/tmp/rocode.sock",
                "/var/run/rocode.sock",
            ];

            for path in candidates {
                if Path::new(path).exists() {
                    return Some(path.to_string());
                }
            }

            // Default to /tmp/rocode.sock even if it doesn't exist yet
            Some("/tmp/rocode.sock".to_string())
        }

        #[cfg(not(unix))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_falls_back_to_http_when_socket_path_missing() {
        let selector = TransportSelector::new(
            Some("/nonexistent/rocode.sock".to_string()),
            "http://localhost:9090".to_string(),
            None,
        );
        assert_eq!(selector.http_base_url, "http://localhost:9090");
        assert_eq!(
            selector.unix_socket_path,
            Some("/nonexistent/rocode.sock".to_string())
        );
    }

    #[test]
    fn selector_uses_http_directly_when_no_socket_path() {
        let selector = TransportSelector::new(None, "http://localhost:9090".to_string(), None);
        assert!(selector.unix_socket_path.is_none());
        assert_eq!(selector.http_base_url, "http://localhost:9090");
    }

    #[test]
    fn test_default_unix_socket_path() {
        let path = TransportSelector::default_unix_socket_path();
        #[cfg(unix)]
        assert!(path.is_some());
        #[cfg(not(unix))]
        assert!(path.is_none());
    }

    #[tokio::test]
    async fn test_selector_fallback_to_http_when_socket_does_not_exist() {
        let selector = TransportSelector::new(
            Some("/nonexistent/socket.sock".to_string()),
            "http://localhost:3000".to_string(),
            None,
        );

        let transport = selector.select().await.unwrap();

        // Should fallback to HTTP since Unix socket doesn't exist
        assert!(
            matches!(transport, FrontendTransport::Http(_)),
            "Expected HTTP transport when socket path does not exist"
        );
    }

    #[tokio::test]
    async fn test_selector_unix_required_errors_when_socket_does_not_exist() {
        let selector = TransportSelector::new(
            Some("/nonexistent/socket.sock".to_string()),
            "http://localhost:3000".to_string(),
            None,
        );

        let error = selector.select_unix_required().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unix socket path does not exist"),
            "unexpected error: {error}"
        );
    }
}
