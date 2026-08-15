#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub enabled: bool,
    pub preflight_enabled: bool,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_cooldown_secs: u64,
    pub rate_limit_rps: f64,
    pub max_inflight: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preflight_enabled: false,
            circuit_breaker_threshold: 0,
            circuit_breaker_cooldown_secs: 30,
            rate_limit_rps: 0.0,
            max_inflight: 0,
        }
    }
}
