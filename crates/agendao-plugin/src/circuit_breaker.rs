use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct CircuitBreaker {
    pub(crate) failures: VecDeque<Instant>,
    threshold: usize,
    window: Duration,
    tripped_until: Option<Instant>,
    cooldown: Duration,
}

impl CircuitBreaker {
    pub fn new(threshold: usize, cooldown: Duration) -> Self {
        Self {
            failures: VecDeque::new(),
            threshold,
            window: Duration::from_secs(60),
            tripped_until: None,
            cooldown,
        }
    }

    pub fn is_tripped(&self) -> bool {
        if let Some(until) = self.tripped_until {
            if Instant::now() < until {
                return true;
            }
        }
        false
    }

    pub fn record_failure(&mut self) {
        let now = Instant::now();
        self.failures.push_back(now);
        while self
            .failures
            .front()
            .is_some_and(|t| now.duration_since(*t) > self.window)
        {
            self.failures.pop_front();
        }
        if self.failures.len() >= self.threshold {
            self.tripped_until = Some(now + self.cooldown);
            tracing::warn!(
                threshold = self.threshold,
                cooldown_secs = self.cooldown.as_secs(),
                "[plugin-breaker] circuit breaker tripped"
            );
        }
    }

    pub fn record_success(&mut self) {
        self.failures.clear();
        self.tripped_until = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trips_after_threshold_failures() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        assert!(!cb.is_tripped());
        cb.record_failure();
        assert!(cb.is_tripped());
    }

    #[test]
    fn recovers_after_cooldown() {
        let mut cb = CircuitBreaker::new(3, Duration::from_millis(50));
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        assert!(cb.is_tripped());
        std::thread::sleep(Duration::from_millis(80));
        assert!(!cb.is_tripped());
    }

    #[test]
    fn resets_on_success() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert_eq!(cb.failures.len(), 0);
    }
}
