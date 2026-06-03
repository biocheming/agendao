use crate::runtime::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use crate::runtime::config::RuntimeConfig;
use crate::runtime::rate_limiter::{RateLimiter, RateLimiterConfig};
use crate::ProviderError;
use reqwest::header::HeaderMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct PreflightGuard {
    rate_limiter: Option<Arc<RateLimiter>>,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
    inflight: Option<Arc<Semaphore>>,
}

impl PreflightGuard {
    pub fn from_config(config: &RuntimeConfig) -> Self {
        let rate_limiter = if config.rate_limit_rps > 0.0 {
            RateLimiterConfig::from_rps(config.rate_limit_rps)
                .map(RateLimiter::new)
                .map(Arc::new)
        } else {
            None
        };

        let circuit_breaker = if config.circuit_breaker_threshold > 0 {
            let cfg = CircuitBreakerConfig {
                failure_threshold: config.circuit_breaker_threshold,
                cooldown: Duration::from_secs(config.circuit_breaker_cooldown_secs.max(1)),
            };
            Some(Arc::new(CircuitBreaker::new(cfg)))
        } else {
            None
        };

        let inflight = if config.max_inflight > 0 {
            Some(Arc::new(Semaphore::new(config.max_inflight as usize)))
        } else {
            None
        };

        Self {
            rate_limiter,
            circuit_breaker,
            inflight,
        }
    }

    pub async fn check(&self) -> Result<Option<OwnedSemaphorePermit>, ProviderError> {
        if let Some(rate_limiter) = &self.rate_limiter {
            rate_limiter.acquire().await?;
        }
        if let Some(circuit_breaker) = &self.circuit_breaker {
            circuit_breaker.allow()?;
        }
        if let Some(inflight) = &self.inflight {
            let permit = inflight
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| ProviderError::ApiError("Backpressure semaphore closed".into()))?;
            return Ok(Some(permit));
        }
        Ok(None)
    }

    pub fn on_success(&self) {
        if let Some(circuit_breaker) = &self.circuit_breaker {
            circuit_breaker.on_success();
        }
    }

    pub fn on_failure(&self) {
        if let Some(circuit_breaker) = &self.circuit_breaker {
            circuit_breaker.on_failure();
        }
    }

    pub async fn update_from_headers(&self, headers: &HeaderMap) {
        let Some(rate_limiter) = &self.rate_limiter else {
            return;
        };

        let retry_after = header_first(headers, &["retry-after"])
            .and_then(|value| value.parse::<u64>().ok().map(Duration::from_secs));
        let remaining = header_first(headers, &["x-ratelimit-remaining-requests"])
            .and_then(|value| value.parse::<u64>().ok());
        let reset_after = header_first(headers, &["x-ratelimit-reset-requests"])
            .and_then(|value| value.parse::<u64>().ok().map(Duration::from_secs));

        if retry_after.is_some() {
            rate_limiter.update_budget(Some(0), retry_after).await;
        } else {
            rate_limiter.update_budget(remaining, reset_after).await;
        }
    }
}

fn header_first(headers: &HeaderMap, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(value) = headers.get(*name) {
            if let Ok(raw) = value.to_str() {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}
