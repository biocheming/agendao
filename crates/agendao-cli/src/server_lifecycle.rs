use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;

#[derive(Clone, Debug, Default)]
pub struct ServerDiscoveryRequest {
    pub port_override: Option<u16>,
    pub cwd: Option<PathBuf>,
    pub unix_socket_path: Option<String>,
}

type DiscoverServerHook = dyn Fn(ServerDiscoveryRequest) -> BoxFuture<'static, anyhow::Result<String>>
    + Send
    + Sync
    + 'static;

#[derive(Clone)]
pub struct FrontendRuntimeContext {
    discover_server: Arc<DiscoverServerHook>,
}

impl FrontendRuntimeContext {
    pub fn new<F>(discover_server: F) -> Self
    where
        F: Fn(ServerDiscoveryRequest) -> BoxFuture<'static, anyhow::Result<String>>
            + Send
            + Sync
            + 'static,
    {
        Self {
            discover_server: Arc::new(discover_server),
        }
    }

    pub fn uninitialized() -> Self {
        Self::new(|_| {
            Box::pin(async {
                anyhow::bail!(
                    "agendao-cli server discovery is not initialized. Run commands through the `agendao` product shell or set AGENDAO_SERVER_URL explicitly."
                )
            })
        })
    }

    pub async fn discover_or_start_server(
        &self,
        port_override: Option<u16>,
    ) -> anyhow::Result<String> {
        self.discover_or_start_server_with_request(ServerDiscoveryRequest {
            port_override,
            cwd: None,
            unix_socket_path: None,
        })
        .await
    }

    pub async fn discover_or_start_server_with_request(
        &self,
        request: ServerDiscoveryRequest,
    ) -> anyhow::Result<String> {
        (self.discover_server)(request).await
    }
}
