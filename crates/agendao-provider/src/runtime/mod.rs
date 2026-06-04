pub mod circuit_breaker;
pub mod config;
pub mod context;
pub mod pipeline;
#[cfg(feature = "http-transport")]
pub mod preflight;
pub mod rate_limiter;

pub use config::{runtime_pipeline_enabled, RuntimeConfig};
pub use context::{ProtocolSource, RuntimeContext};
pub use pipeline::Pipeline;
#[cfg(feature = "http-transport")]
pub use preflight::PreflightGuard;
use std::sync::Arc;

pub struct ProviderRuntime {
    pub config: RuntimeConfig,
    pub context: RuntimeContext,
    #[cfg(feature = "http-transport")]
    pub preflight: Option<PreflightGuard>,
    pub pipeline: Option<Arc<Pipeline>>,
}

impl ProviderRuntime {
    pub fn new(config: RuntimeConfig, context: RuntimeContext) -> Self {
        #[cfg(feature = "http-transport")]
        let preflight = if config.enabled && config.preflight_enabled {
            Some(PreflightGuard::from_config(&config))
        } else {
            None
        };
        Self {
            config,
            context,
            #[cfg(feature = "http-transport")]
            preflight,
            pipeline: None,
        }
    }

    pub fn from_config(config: RuntimeConfig, provider_id: impl Into<String>) -> Self {
        let context = RuntimeContext {
            protocol_source: ProtocolSource::BuiltinAdapter {
                npm: "unknown".to_string(),
            },
            provider_id: provider_id.into(),
            created_at: std::time::Instant::now(),
        };
        Self::new(config, context)
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn is_pipeline_enabled(&self) -> bool {
        self.config.enabled && self.config.pipeline_enabled
    }

    pub fn is_preflight_enabled(&self) -> bool {
        #[cfg(feature = "http-transport")]
        {
            self.config.enabled && self.config.preflight_enabled
        }
        #[cfg(not(feature = "http-transport"))]
        {
            false
        }
    }

    pub fn set_pipeline(&mut self, pipeline: Arc<Pipeline>) {
        self.pipeline = Some(pipeline);
    }

    pub fn with_pipeline(mut self, pipeline: Arc<Pipeline>) -> Self {
        self.set_pipeline(pipeline);
        self
    }
}
