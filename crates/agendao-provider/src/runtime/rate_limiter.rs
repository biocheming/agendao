use crate::ProviderError;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    pub rps: f64,
    pub burst: f64,
}

impl RateLimiterConfig {
    pub fn from_rps(rps: f64) -> Option<Self> {
        if !rps.is_finite() || rps < 0.0 {
            return None;
        }
        Some(Self {
            rps,
            burst: rps.max(1.0),
        })
    }
}

#[derive(Debug, Clone)]
pub struct RateLimiterSnapshot {
    pub rps: f64,
    pub burst: f64,
    pub tokens: f64,
    pub estimated_wait_ms: Option<u64>,
}

#[derive(Debug)]
struct RateLimiterState {
    tokens: f64,
    last: Instant,
    blocked_until: Option<Instant>,
    remaining: Option<u64>,
}

pub struct RateLimiter {
    cfg: RateLimiterConfig,
    state: Mutex<RateLimiterState>,
}

impl RateLimiter {
    pub fn new(cfg: RateLimiterConfig) -> Self {
        let burst = cfg.burst;
        Self {
            cfg,
            state: Mutex::new(RateLimiterState {
                tokens: burst,
                last: Instant::now(),
                blocked_until: None,
                remaining: None,
            }),
        }
    }

    fn refill_locked(cfg: &RateLimiterConfig, state: &mut RateLimiterState) {
        let now = Instant::now();
        let elapsed_secs = now.duration_since(state.last).as_secs_f64();
        if elapsed_secs > 0.0 {
            state.tokens = (state.tokens + elapsed_secs * cfg.rps).min(cfg.burst);
            state.last = now;
        }
    }

    pub async fn acquire(&self) -> Result<(), ProviderError> {
        if self.cfg.rps <= 0.0 {
            return Ok(());
        }

        loop {
            let wait = {
                let mut state = self.state.lock().await;
                let now = Instant::now();

                if let Some(until) = state.blocked_until {
                    if until > now {
                        until.duration_since(now)
                    } else {
                        state.blocked_until = None;
                        Duration::from_millis(0)
                    }
                } else {
                    Self::refill_locked(&self.cfg, &mut state);
                    if state.tokens >= 1.0 && state.remaining.unwrap_or(1) > 0 {
                        state.tokens -= 1.0;
                        if let Some(rem) = state.remaining.as_mut() {
                            *rem = rem.saturating_sub(1);
                        }
                        return Ok(());
                    }

                    let missing = (1.0 - state.tokens).max(0.0);
                    Duration::from_secs_f64(missing / self.cfg.rps)
                }
            };
            if wait.as_millis() > 0 {
                tokio::time::sleep(wait).await;
            }
        }
    }

    pub async fn try_acquire(&self) -> bool {
        if self.cfg.rps <= 0.0 {
            return true;
        }
        let mut state = self.state.lock().await;
        Self::refill_locked(&self.cfg, &mut state);
        if state.tokens >= 1.0 && state.remaining.unwrap_or(1) > 0 {
            state.tokens -= 1.0;
            if let Some(rem) = state.remaining.as_mut() {
                *rem = rem.saturating_sub(1);
            }
            true
        } else {
            false
        }
    }

    pub async fn update_budget(&self, remaining: Option<u64>, reset_after: Option<Duration>) {
        let mut state = self.state.lock().await;
        if let Some(rem) = remaining {
            state.remaining = Some(rem);
            if rem == 0 {
                state.blocked_until =
                    Some(Instant::now() + reset_after.unwrap_or(Duration::from_secs(1)));
            } else {
                state.blocked_until = None;
            }
        }
    }

    pub async fn snapshot(&self) -> RateLimiterSnapshot {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let mut wait_ms = None;

        if let Some(until) = state.blocked_until {
            if until > now {
                wait_ms = Some(until.duration_since(now).as_millis() as u64);
            }
        }
        if self.cfg.rps > 0.0 {
            Self::refill_locked(&self.cfg, &mut state);
            if state.tokens < 1.0 {
                let missing = 1.0 - state.tokens;
                let local_wait_ms = (missing / self.cfg.rps * 1000.0) as u64;
                wait_ms = Some(wait_ms.unwrap_or(0).max(local_wait_ms));
            }
        }

        RateLimiterSnapshot {
            rps: self.cfg.rps,
            burst: self.cfg.burst,
            tokens: state.tokens,
            estimated_wait_ms: wait_ms,
        }
    }
}
