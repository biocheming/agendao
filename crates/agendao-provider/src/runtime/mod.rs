pub mod circuit_breaker;
pub mod config;
#[cfg(feature = "http-transport")]
pub mod preflight;
pub mod rate_limiter;

pub use config::RuntimeConfig;
#[cfg(feature = "http-transport")]
pub use preflight::PreflightGuard;
pub struct ProviderRuntime {
    pub config: RuntimeConfig,
    #[cfg(feature = "http-transport")]
    pub preflight: Option<PreflightGuard>,
}

impl ProviderRuntime {
    pub fn new(config: RuntimeConfig) -> Self {
        #[cfg(feature = "http-transport")]
        let preflight = if config.enabled && config.preflight_enabled {
            Some(PreflightGuard::from_config(&config))
        } else {
            None
        };
        Self {
            config,
            #[cfg(feature = "http-transport")]
            preflight,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
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
}
