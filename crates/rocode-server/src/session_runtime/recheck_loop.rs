use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::telemetry::RuntimeTelemetryAuthority;

/// Default scan interval: 30 seconds.
const RECHECK_SCAN_INTERVAL_SECS: u64 = 30;

/// Spawn a background loop that periodically scans blocked and sleeping
/// sessions. Sessions whose `recheck_at` or `wake_at` has passed are
/// automatically transitioned to `Idle` via the telemetry authority.
///
/// The loop runs at a fixed interval and uses only the existing
/// `recheck_session()` / `wake_session()` methods, which go through
/// `set_session_run_status()` — so control, projection, and event bus
/// all stay consistent.
pub fn spawn_recheck_wake_loop(
    telemetry: Arc<RuntimeTelemetryAuthority>,
    cancel: CancellationToken,
) {
    let interval_secs = RECHECK_SCAN_INTERVAL_SECS;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        // Skip the first tick (fires immediately).
        interval.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {},
            }

            run_recheck_wake_tick(&telemetry).await;
        }
    });
}

/// Perform a single scan-and-transition tick. Exposed for testing.
async fn run_recheck_wake_tick(telemetry: &RuntimeTelemetryAuthority) {
    let statuses = telemetry.session_run_statuses().await;
    let mut rechecked = 0usize;
    let mut woken = 0usize;

    for (session_id, status) in &statuses {
        match status {
            crate::runtime_control::SessionRunStatus::Blocked { .. } => {
                if telemetry.recheck_session(session_id).await.is_some() {
                    rechecked += 1;
                    tracing::info!(%session_id, "auto-rechecked blocked session");
                }
            }
            crate::runtime_control::SessionRunStatus::Sleeping { .. } => {
                if telemetry.wake_session(session_id).await.is_some() {
                    woken += 1;
                    tracing::info!(%session_id, "auto-woken sleeping session");
                }
            }
            _ => {}
        }
    }

    if rechecked > 0 || woken > 0 {
        tracing::info!(
            rechecked,
            woken,
            total = statuses.len(),
            "recheck/wake loop tick"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_control::SessionRunStatus;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    use super::run_recheck_wake_tick;

    #[tokio::test]
    async fn tick_scans_and_transitions_past_due_sessions() {
        let (tx, _rx) = broadcast::channel(1);
        let telemetry = Arc::new(RuntimeTelemetryAuthority::new(tx, None));

        // Create one blocked (past-due) and one sleeping (past-due).
        telemetry
            .set_session_run_status(
                "tick-blocked",
                SessionRunStatus::Blocked {
                    reason: Some("due".to_string()),
                    recheck_at: Some(1),
                },
            )
            .await;
        telemetry
            .set_session_run_status(
                "tick-sleeping",
                SessionRunStatus::Sleeping {
                    reason: Some("due".to_string()),
                    wake_at: Some(1),
                },
            )
            .await;
        // Create one blocked with future recheck_at — should NOT transition.
        telemetry
            .set_session_run_status(
                "tick-future",
                SessionRunStatus::Blocked {
                    reason: Some("future".to_string()),
                    recheck_at: Some(9999999999999i64),
                },
            )
            .await;

        // Run one tick.
        run_recheck_wake_tick(&telemetry).await;

        let statuses = telemetry.session_run_statuses().await;
        // Past-due blocked → Idle (or gone from statuses).
        assert!(
            !matches!(statuses.get("tick-blocked"), Some(SessionRunStatus::Blocked { .. })),
            "past-due blocked session should transition out of Blocked"
        );
        // Past-due sleeping → Idle (or gone).
        assert!(
            !matches!(statuses.get("tick-sleeping"), Some(SessionRunStatus::Sleeping { .. })),
            "past-due sleeping session should transition out of Sleeping"
        );
        // Future blocked → still Blocked.
        assert!(
            matches!(statuses.get("tick-future"), Some(SessionRunStatus::Blocked { .. })),
            "future blocked session should stay Blocked"
        );
    }

    #[tokio::test]
    async fn auto_recheck_loop_triggers_past_due_blocked_session() {
        let (tx, _rx) = broadcast::channel(1);
        let telemetry = Arc::new(RuntimeTelemetryAuthority::new(tx, None));
        let sid = "auto-recheck-test";

        // Set blocked with past recheck_at.
        telemetry
            .set_session_run_status(
                sid,
                SessionRunStatus::Blocked {
                    reason: Some("test".to_string()),
                    recheck_at: Some(1),
                },
            )
            .await;

        // Manually trigger one scan tick (bypass interval).
        let statuses = telemetry.session_run_statuses().await;
        for (session_id, status) in &statuses {
            if matches!(status, SessionRunStatus::Blocked { .. }) {
                assert!(telemetry.recheck_session(session_id).await.is_some());
            }
        }

        // Verify session is now Idle.
        let status = telemetry.session_run_statuses().await;
        assert!(
            matches!(status.get(sid), Some(SessionRunStatus::Idle) | None),
            "blocked session should transition to Idle after recheck"
        );
    }

    #[tokio::test]
    async fn auto_recheck_loop_skips_future_recheck_at() {
        let (tx, _rx) = broadcast::channel(1);
        let telemetry = Arc::new(RuntimeTelemetryAuthority::new(tx, None));
        let sid = "auto-recheck-future";

        telemetry
            .set_session_run_status(
                sid,
                SessionRunStatus::Blocked {
                    reason: Some("future".to_string()),
                    recheck_at: Some(9999999999999i64),
                },
            )
            .await;

        // recheck_session should return None (not due yet).
        let result = telemetry.recheck_session(sid).await;
        assert!(result.is_none(), "recheck should not fire before recheck_at");

        // Session should still be Blocked.
        let status = telemetry.session_run_statuses().await;
        assert!(
            matches!(status.get(sid), Some(SessionRunStatus::Blocked { .. })),
            "blocked session should stay blocked when recheck_at is in the future"
        );
    }

    #[tokio::test]
    async fn auto_wake_loop_triggers_past_due_sleeping_session() {
        let (tx, _rx) = broadcast::channel(1);
        let telemetry = Arc::new(RuntimeTelemetryAuthority::new(tx, None));
        let sid = "auto-wake-test";

        telemetry
            .set_session_run_status(
                sid,
                SessionRunStatus::Sleeping {
                    reason: Some("test".to_string()),
                    wake_at: Some(1),
                },
            )
            .await;

        let result = telemetry.wake_session(sid).await;
        assert!(result.is_some(), "wake should succeed");

        let status = telemetry.session_run_statuses().await;
        assert!(
            matches!(status.get(sid), Some(SessionRunStatus::Idle) | None),
            "sleeping session should transition to Idle after wake"
        );
    }
}
