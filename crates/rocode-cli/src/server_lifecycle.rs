use std::sync::Arc;

use futures::future::BoxFuture;

type DiscoverServerHook =
    dyn Fn(Option<u16>) -> BoxFuture<'static, anyhow::Result<String>> + Send + Sync + 'static;

#[derive(Clone)]
pub struct FrontendRuntimeContext {
    discover_server: Arc<DiscoverServerHook>,
}

impl FrontendRuntimeContext {
    pub fn new<F>(discover_server: F) -> Self
    where
        F: Fn(Option<u16>) -> BoxFuture<'static, anyhow::Result<String>> + Send + Sync + 'static,
    {
        Self {
            discover_server: Arc::new(discover_server),
        }
    }

    pub fn uninitialized() -> Self {
        Self::new(|_| {
            Box::pin(async {
                anyhow::bail!(
                    "rocode-cli server discovery is not initialized. Run commands through the `rocode` product shell or set ROCODE_SERVER_URL explicitly."
                )
            })
        })
    }

    pub async fn discover_or_start_server(
        &self,
        port_override: Option<u16>,
    ) -> anyhow::Result<String> {
        (self.discover_server)(port_override).await
    }
}
