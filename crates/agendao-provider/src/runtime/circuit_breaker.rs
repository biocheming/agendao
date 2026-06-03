use crate::ProviderError;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub cooldown: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cooldown: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerSnapshot {
    pub failure_threshold: u32,
    pub cooldown_ms: u64,
    pub consecutive_failures: u32,
    pub open_remaining_ms: Option<u64>,
}

#[derive(Debug)]
struct BreakerState {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

pub struct CircuitBreaker {
    cfg: CircuitBreakerConfig,
    state: std::sync::Mutex<BreakerState>,
}

impl CircuitBreaker {
    pub fn new(cfg: CircuitBreakerConfig) -> Self {
        Self {
            cfg,
            state: std::sync::Mutex::new(BreakerState {
                consecutive_failures: 0,
                open_until: None,
            }),
        }
    }

    pub fn allow(&self) -> Result<(), ProviderError> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProviderError::ApiError("CircuitBreaker lock poisoned".into()))?;
        if let Some(until) = state.open_until {
            if now < until {
                return Err(ProviderError::ApiError("circuit breaker open".into()));
            }
            state.open_until = None;
            state.consecutive_failures = 0;
        }
        Ok(())
    }

    pub fn on_success(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.consecutive_failures = 0;
            state.open_until = None;
        }
    }

    pub fn on_failure(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
            if state.consecutive_failures >= self.cfg.failure_threshold {
                state.open_until = Some(Instant::now() + self.cfg.cooldown);
            }
        }
    }

    pub fn snapshot(&self) -> CircuitBreakerSnapshot {
        let now = Instant::now();
        if let Ok(state) = self.state.lock() {
            let open_remaining_ms = state.open_until.and_then(|until| {
                if until > now {
                    Some((until - now).as_millis() as u64)
                } else {
                    None
                }
            });
            CircuitBreakerSnapshot {
                failure_threshold: self.cfg.failure_threshold,
                cooldown_ms: self.cfg.cooldown.as_millis() as u64,
                consecutive_failures: state.consecutive_failures,
                open_remaining_ms,
            }
        } else {
            CircuitBreakerSnapshot {
                failure_threshold: self.cfg.failure_threshold,
                cooldown_ms: self.cfg.cooldown.as_millis() as u64,
                consecutive_failures: 0,
                open_remaining_ms: None,
            }
        }
    }
}
