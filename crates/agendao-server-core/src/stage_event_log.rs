//! Stage Event Log — per-session in-memory buffer of [`StageEvent`]s with
//! query-by-session / query-by-stage / replay support.
//!
//! # Architecture
//!
//! The log is an `Arc`-friendly struct stored once in `ServerState`.
//! Internally it keeps a `RwLock<HashMap<session_id, VecDeque<StageEvent>>>`
//! with a configurable per-session capacity (default 4096).  When the capacity
//! is exceeded, the oldest events are evicted.
//!
//! Recording and querying are both lock-granular: writers take a write-lock
//! only long enough to push; readers take a read-lock and filter in-place.

use agendao_stage_protocol::StageEvent;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tokio::sync::RwLock;

/// Default maximum events kept per session before oldest are evicted.
const DEFAULT_MAX_EVENTS_PER_SESSION: usize = 4096;

/// Query filter for retrieving events from the log.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EventFilter {
    /// Only return events belonging to this stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    /// Only return events belonging to this execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    /// Only return events of this type (e.g. `"execution.topology.changed"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    /// Only return events with `ts >= since` (epoch millis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    /// Maximum number of events to return (applied *after* offset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Number of matching events to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

/// Per-session in-memory event buffer with capped capacity.
pub struct StageEventLog {
    /// session_id → ring buffer of events.
    sessions: RwLock<HashMap<String, VecDeque<StageEvent>>>,
    /// Maximum events retained per session.
    max_per_session: usize,
}

impl StageEventLog {
    /// Create a new log with the default per-session capacity.
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_per_session: DEFAULT_MAX_EVENTS_PER_SESSION,
        }
    }

    /// Create a new log with a custom per-session capacity.
    pub fn with_capacity(max_per_session: usize) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_per_session,
        }
    }

    /// Record an event into the given session's buffer.
    ///
    /// If the buffer exceeds `max_per_session`, the oldest event is evicted.
    pub async fn record(&self, session_id: &str, event: StageEvent) {
        let mut guard = self.sessions.write().await;
        let buf = guard
            .entry(session_id.to_string())
            .or_insert_with(VecDeque::new);
        buf.push_back(event);
        while buf.len() > self.max_per_session {
            buf.pop_front();
        }
    }

    /// Query events for a session, applying the given filter.
    ///
    /// Returns an owned `Vec` of matching events in chronological order.
    pub async fn query(&self, session_id: &str, filter: &EventFilter) -> Vec<StageEvent> {
        let guard = self.sessions.read().await;
        let Some(buf) = guard.get(session_id) else {
            return Vec::new();
        };

        let iter = buf.iter().filter(|evt| {
            if let Some(ref sid) = filter.stage_id {
                if evt.stage_id.as_ref() != Some(sid) {
                    return false;
                }
            }
            if let Some(ref eid) = filter.execution_id {
                if evt.execution_id.as_ref() != Some(eid) {
                    return false;
                }
            }
            if let Some(ref et) = filter.event_type {
                if evt.event_type != *et {
                    return false;
                }
            }
            if let Some(since) = filter.since {
                if evt.ts < since {
                    return false;
                }
            }
            true
        });

        let offset = filter.offset.unwrap_or(0);
        let limited: Box<dyn Iterator<Item = &StageEvent>> = match filter.limit {
            Some(lim) => Box::new(iter.skip(offset).take(lim)),
            None => Box::new(iter.skip(offset)),
        };

        limited.cloned().collect()
    }

    /// Return the distinct `stage_id` values present in a session's log.
    pub async fn stage_ids(&self, session_id: &str) -> Vec<String> {
        let guard = self.sessions.read().await;
        let Some(buf) = guard.get(session_id) else {
            return Vec::new();
        };

        let mut seen = std::collections::HashSet::new();
        let mut ids = Vec::new();
        for evt in buf.iter() {
            if let Some(ref sid) = evt.stage_id {
                if seen.insert(sid.clone()) {
                    ids.push(sid.clone());
                }
            }
        }
        ids
    }

    /// Remove all events for a session.
    pub async fn clear_session(&self, session_id: &str) {
        let mut guard = self.sessions.write().await;
        guard.remove(session_id);
    }
}

impl Default for StageEventLog {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_stage_protocol::EventScope;

    fn make_event(
        stage_id: Option<&str>,
        execution_id: Option<&str>,
        event_type: &str,
        ts: i64,
    ) -> StageEvent {
        StageEvent {
            event_id: format!("evt_test_{}", ts),
            scope: EventScope::Stage,
            stage_id: stage_id.map(String::from),
            execution_id: execution_id.map(String::from),
            event_type: event_type.to_string(),
            ts,
            payload: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn record_and_query_all_events() {
        let log = StageEventLog::new();
        log.record(
            "s1",
            make_event(Some("stg1"), Some("e1"), "tool.start", 100),
        )
        .await;
        log.record("s1", make_event(Some("stg1"), Some("e2"), "tool.end", 200))
            .await;
        log.record(
            "s1",
            make_event(Some("stg2"), Some("e3"), "agent.start", 300),
        )
        .await;

        let all = log.query("s1", &EventFilter::default()).await;
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].ts, 100);
        assert_eq!(all[2].ts, 300);
    }

    #[tokio::test]
    async fn query_filters_by_stage_id() {
        let log = StageEventLog::new();
        log.record("s1", make_event(Some("stg1"), None, "a", 1))
            .await;
        log.record("s1", make_event(Some("stg2"), None, "b", 2))
            .await;
        log.record("s1", make_event(Some("stg1"), None, "c", 3))
            .await;

        let filtered = log
            .query(
                "s1",
                &EventFilter {
                    stage_id: Some("stg1".into()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(filtered.len(), 2);
        assert!(filtered
            .iter()
            .all(|e| e.stage_id.as_deref() == Some("stg1")));
    }

    #[tokio::test]
    async fn query_filters_by_execution_id() {
        let log = StageEventLog::new();
        log.record("s1", make_event(None, Some("ex_a"), "x", 1))
            .await;
        log.record("s1", make_event(None, Some("ex_b"), "y", 2))
            .await;

        let filtered = log
            .query(
                "s1",
                &EventFilter {
                    execution_id: Some("ex_a".into()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].execution_id.as_deref(), Some("ex_a"));
    }

    #[tokio::test]
    async fn query_filters_by_event_type() {
        let log = StageEventLog::new();
        log.record("s1", make_event(None, None, "tool.start", 1))
            .await;
        log.record("s1", make_event(None, None, "tool.end", 2))
            .await;
        log.record("s1", make_event(None, None, "tool.start", 3))
            .await;

        let filtered = log
            .query(
                "s1",
                &EventFilter {
                    event_type: Some("tool.start".into()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(filtered.len(), 2);
    }

    #[tokio::test]
    async fn query_filters_by_since_timestamp() {
        let log = StageEventLog::new();
        log.record("s1", make_event(None, None, "a", 100)).await;
        log.record("s1", make_event(None, None, "b", 200)).await;
        log.record("s1", make_event(None, None, "c", 300)).await;

        let filtered = log
            .query(
                "s1",
                &EventFilter {
                    since: Some(200),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].ts, 200);
    }

    #[tokio::test]
    async fn query_applies_limit_and_offset() {
        let log = StageEventLog::new();
        for i in 0..10 {
            log.record("s1", make_event(None, None, "evt", i)).await;
        }

        let page = log
            .query(
                "s1",
                &EventFilter {
                    offset: Some(3),
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].ts, 3);
        assert_eq!(page[1].ts, 4);
    }

    #[tokio::test]
    async fn capacity_eviction_drops_oldest_events() {
        let log = StageEventLog::with_capacity(3);
        for i in 0..5 {
            log.record("s1", make_event(None, None, "evt", i)).await;
        }

        let all = log.query("s1", &EventFilter::default()).await;
        assert_eq!(all.len(), 3);
        // Oldest two (ts=0, ts=1) should have been evicted.
        assert_eq!(all[0].ts, 2);
        assert_eq!(all[1].ts, 3);
        assert_eq!(all[2].ts, 4);
    }

    #[tokio::test]
    async fn clear_session_removes_all_events() {
        let log = StageEventLog::new();
        log.record("s1", make_event(None, None, "a", 1)).await;
        log.record("s1", make_event(None, None, "b", 2)).await;
        log.clear_session("s1").await;

        let all = log.query("s1", &EventFilter::default()).await;
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn sessions_are_isolated() {
        let log = StageEventLog::new();
        log.record("s1", make_event(None, None, "a", 1)).await;
        log.record("s2", make_event(None, None, "b", 2)).await;

        let s1 = log.query("s1", &EventFilter::default()).await;
        let s2 = log.query("s2", &EventFilter::default()).await;
        assert_eq!(s1.len(), 1);
        assert_eq!(s2.len(), 1);
        assert_eq!(s1[0].event_type, "a");
        assert_eq!(s2[0].event_type, "b");
    }

    #[tokio::test]
    async fn stage_ids_returns_distinct_ordered_by_first_appearance() {
        let log = StageEventLog::new();
        log.record("s1", make_event(Some("stg_b"), None, "x", 1))
            .await;
        log.record("s1", make_event(Some("stg_a"), None, "x", 2))
            .await;
        log.record("s1", make_event(Some("stg_b"), None, "x", 3))
            .await;
        log.record("s1", make_event(None, None, "x", 4)).await;

        let ids = log.stage_ids("s1").await;
        assert_eq!(ids, vec!["stg_b", "stg_a"]);
    }

    #[tokio::test]
    async fn query_nonexistent_session_returns_empty() {
        let log = StageEventLog::new();
        let result = log.query("nonexistent", &EventFilter::default()).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn combined_filters_intersect() {
        let log = StageEventLog::new();
        log.record(
            "s1",
            make_event(Some("stg1"), Some("e1"), "tool.start", 100),
        )
        .await;
        log.record("s1", make_event(Some("stg1"), Some("e2"), "tool.end", 200))
            .await;
        log.record(
            "s1",
            make_event(Some("stg2"), Some("e1"), "tool.start", 300),
        )
        .await;

        let filtered = log
            .query(
                "s1",
                &EventFilter {
                    stage_id: Some("stg1".into()),
                    event_type: Some("tool.start".into()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].ts, 100);
    }
}
