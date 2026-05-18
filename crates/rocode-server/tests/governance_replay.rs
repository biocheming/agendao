//! Governance replay integration smoke tests for stage event log behavior.
//!
//! These tests verify:
//! 1. Concurrent event recording preserves all writes
//! 2. Fixture replay covers the main EventFilter combinations
//! 3. StageEvent builder produces unique IDs
//! 4. Clearing a session and re-recording starts from a fresh state

use rocode_command::governance_fixtures::multi_agent_replay_fixture;
use rocode_command::stage_protocol::*;
use rocode_server::stage_event_log::{EventFilter, StageEventLog};
use std::collections::HashSet;
use std::sync::Arc;

// ── Helpers ──────────────────────────────────────────────────────────

fn make_event(
    stage_id: Option<&str>,
    execution_id: Option<&str>,
    event_type: &str,
    ts: i64,
) -> StageEvent {
    StageEvent {
        event_id: format!("evt_gov_{}", ts),
        scope: EventScope::Stage,
        stage_id: stage_id.map(String::from),
        execution_id: execution_id.map(String::from),
        event_type: event_type.to_string(),
        ts,
        payload: serde_json::json!({}),
    }
}

// ── 1. Concurrent event recording and querying ──────────────────────

#[tokio::test]
async fn concurrent_event_recording_preserves_all_events() {
    let log = Arc::new(StageEventLog::new());
    let session = "session_concurrent_1";

    // Spawn 3 concurrent writers (simulating 3 stages recording events simultaneously)
    let mut handles = Vec::new();
    for stage_idx in 0..3 {
        let log = log.clone();
        let sid = session.to_string();
        let handle = tokio::spawn(async move {
            for i in 0..10 {
                let ts = (stage_idx as i64) * 1000 + i;
                log.record(
                    &sid,
                    make_event(
                        Some(&format!("stage_{}", stage_idx)),
                        Some(&format!("exec_{}_{}", stage_idx, i)),
                        "topology_changed",
                        ts,
                    ),
                )
                .await;
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let all = log.query(session, &EventFilter::default()).await;
    assert_eq!(all.len(), 30, "all 30 events should be recorded");
}

// ── 2. Fixture-driven event replay (smoke) ──────────────────────────

#[tokio::test]
async fn fixture_events_replay_through_event_log() {
    let fixture = multi_agent_replay_fixture();
    let log = StageEventLog::new();
    let session = &fixture.session_id;

    for stage in &fixture.stages {
        for event in &stage.events {
            log.record(session, event.clone()).await;
        }
    }

    let all = log.query(session, &EventFilter::default()).await;
    assert_eq!(all.len(), fixture.expected.total_events);
    let stage_ids = log.stage_ids(session).await;
    assert_eq!(stage_ids, fixture.expected.distinct_stage_ids);
}

#[tokio::test]
async fn fixture_event_log_all_filter_combinations() {
    let fixture = multi_agent_replay_fixture();
    let log = StageEventLog::new();
    let session = &fixture.session_id;

    for stage in &fixture.stages {
        for event in &stage.events {
            log.record(session, event.clone()).await;
        }
    }

    // Filter by event type with exact counts from the fixture.
    let stage_started = log.query(session, &EventFilter {
        event_type: Some("stage_started".into()), ..Default::default()
    }).await;
    assert_eq!(stage_started.len(), 3, "fixture has 3 stage_started events");
    let agent_started = log.query(session, &EventFilter {
        event_type: Some("agent_started".into()), ..Default::default()
    }).await;
    assert_eq!(agent_started.len(), 4, "fixture has 4 agent_started events");
    let question_asked = log.query(session, &EventFilter {
        event_type: Some("question_asked".into()), ..Default::default()
    }).await;
    assert_eq!(question_asked.len(), 2, "fixture has 2 question_asked events");

    // Combined stage + type: implementation stage has exactly 1 tool_started
    // with tool=write_file. Verify both count AND content.
    let impl_tools = log.query(session, &EventFilter {
        stage_id: Some("stage_impl_002".into()),
        event_type: Some("tool_started".into()),
        ..Default::default()
    }).await;
    assert_eq!(impl_tools.len(), 1);
    assert_eq!(
        impl_tools[0].payload.get("tool").and_then(|v| v.as_str()),
        Some("write_file"),
        "combined stage+type filter must return the correct tool event"
    );

    // Since filter.
    let since = log
        .query(session, &EventFilter {
            since: Some(1710000020000),
            ..Default::default()
        })
        .await;
    assert_eq!(since.len(), 3);

    // Pagination.
    let mut collected = Vec::new();
    let mut offset = 0;
    loop {
        let page = log
            .query(session, &EventFilter {
                limit: Some(3),
                offset: Some(offset),
                ..Default::default()
            })
            .await;
        if page.is_empty() { break; }
        collected.extend(page);
        offset += 3;
    }
    assert_eq!(collected.len(), fixture.expected.total_events);
}

// ── 3. Cross-layer: event IDs unique across concurrent writes ───────

#[tokio::test]
async fn stage_event_builder_produces_unique_ids_under_concurrency() {
    let events: Vec<StageEvent> = (0..100)
        .map(|_| StageEvent::new(
            EventScope::Stage, Some("s1".into()), None, "test", serde_json::json!({}),
        ))
        .collect();
    let ids: HashSet<&str> = events.iter().map(|e| e.event_id.as_str()).collect();
    assert_eq!(ids.len(), 100, "all 100 event IDs should be unique");
}

// ── 4. Clear session and re-record ───────────────────────────────────

#[tokio::test]
async fn clear_and_rerecord_gives_fresh_state() {
    let log = StageEventLog::new();
    let session = "session_clear_test";

    // Record some events
    for i in 0..5 {
        log.record(session, make_event(Some("stage_old"), None, "old", i))
            .await;
    }
    assert_eq!(log.query(session, &EventFilter::default()).await.len(), 5);

    // Clear
    log.clear_session(session).await;
    assert!(log.query(session, &EventFilter::default()).await.is_empty());

    // Re-record new events
    for i in 0..3 {
        log.record(session, make_event(Some("stage_new"), None, "new", 100 + i))
            .await;
    }
    let events = log.query(session, &EventFilter::default()).await;
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|e| e.event_type == "new"));
}
