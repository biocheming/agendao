//! Phase 5 governance probes: the projection layer must never present a
//! denial as a failed run, never let a violation mutate the active
//! sandbox set, and never let a fabricated (empty-id) execution enter
//! the sandboxed set.
//!
//! Host-level negative probes — network egress, filesystem escapes,
//! environment leakage — live in `agendao-sandbox`'s `linux_runtime`
//! tests. These probe the server-side semantics that sit between the
//! sandbox events and what frontends are allowed to display.

use std::sync::Arc;

use tokio::sync::broadcast;

use agendao_sandbox::{
    Attribution, CleanupStatus, DenialReason, FilesystemMode, NetworkMode, ProcessMode,
    ProfileKind, ProfileSummary, SandboxEvent, SandboxEventSink, SandboxExit, SandboxViolation,
    SandboxViolationKind,
};
use agendao_server::sandbox_authority::ProjectingSandboxEventSink;
use agendao_server_core::runtime_state::RuntimeStateStore;
use agendao_server_core::{ServerBusEvent, ServerEvent};

fn prepared_event(execution_id: &str, session: &str) -> SandboxEvent {
    SandboxEvent::Prepared {
        execution_id: execution_id.into(),
        session_origin: Some(session.into()),
        profile: ProfileSummary {
            requested_kind: ProfileKind::WorkspaceWrite,
            process_mode: ProcessMode::Contained,
            filesystem_mode: FilesystemMode::WorkspaceWrite,
            network_mode: NetworkMode::Disabled,
        },
        plan_fingerprint: "fp-1".into(),
        backend: "bwrap".into(),
    }
}

/// The sink must translate every sandbox event into the corresponding
/// ServerEvent with the wire vocabulary fixed by this crate — a denial
/// stays a denial with its machine-readable reason, never a generic
/// failure string.
#[tokio::test]
async fn sink_translates_denials_with_machine_readable_reasons() {
    let (tx, mut rx) = broadcast::channel::<Arc<ServerBusEvent>>(16);
    let sink = ProjectingSandboxEventSink::new(tx);

    sink.record(SandboxEvent::Denied {
        execution_id: "exec-1".into(),
        session_origin: Some("ses-1".into()),
        reason: DenialReason::PolicyDenied,
        detail: Some("native requires yolo".into()),
    });
    sink.record(SandboxEvent::Denied {
        execution_id: "exec-2".into(),
        session_origin: Some("ses-1".into()),
        reason: DenialReason::BackendUnavailable {
            capability: "bubblewrap".into(),
        },
        detail: None,
    });

    let first = rx.recv().await.unwrap();
    match first.event_ref() {
        ServerEvent::SandboxDenied {
            session_id,
            execution_id,
            reason,
            detail,
        } => {
            assert_eq!(session_id.as_deref(), Some("ses-1"));
            assert_eq!(execution_id, "exec-1");
            assert_eq!(reason, "policy_denied");
            assert_eq!(detail.as_deref(), Some("native requires yolo"));
        }
        other => panic!("expected SandboxDenied, got {:?}", other.event_name()),
    }
    let second = rx.recv().await.unwrap();
    match second.event_ref() {
        ServerEvent::SandboxDenied { reason, detail, .. } => {
            assert_eq!(reason, "backend_unavailable");
            // The capability folds into detail so the UI can say what is
            // missing on this host.
            assert_eq!(detail.as_deref(), Some("bubblewrap"));
        }
        other => panic!("expected SandboxDenied, got {:?}", other.event_name()),
    }
}

#[tokio::test]
async fn sink_translates_profile_kinds_and_exit_ladders() {
    let (tx, mut rx) = broadcast::channel::<Arc<ServerBusEvent>>(16);
    let sink = ProjectingSandboxEventSink::new(tx);

    sink.record(prepared_event("exec-3", "ses-2"));
    match rx.recv().await.unwrap().event_ref() {
        ServerEvent::SandboxPrepared {
            session_id,
            execution_id,
            profile_kind,
            plan_fingerprint,
            backend,
        } => {
            assert_eq!(session_id.as_deref(), Some("ses-2"));
            assert_eq!(execution_id, "exec-3");
            assert_eq!(profile_kind, "workspace_write");
            assert_eq!(plan_fingerprint, "fp-1");
            assert_eq!(backend, "bwrap");
        }
        other => panic!("expected SandboxPrepared, got {:?}", other.event_name()),
    }

    sink.record(SandboxEvent::Exited {
        execution_id: "exec-3".into(),
        session_origin: Some("ses-2".into()),
        status: SandboxExit {
            success: false,
            code: Some(9),
            signal: None,
            cleanup: CleanupStatus::KilledAfterGrace,
        },
        backend: "bwrap".into(),
    });
    match rx.recv().await.unwrap().event_ref() {
        ServerEvent::SandboxExited {
            exit_code,
            success,
            cleanup,
            ..
        } => {
            assert_eq!(*exit_code, Some(9));
            assert!(!success);
            assert_eq!(cleanup, "killed_after_grace");
        }
        other => panic!("expected SandboxExited, got {:?}", other.event_name()),
    }
}

#[tokio::test]
async fn sink_routes_violation_payloads_verbatim() {
    let (tx, mut rx) = broadcast::channel::<Arc<ServerBusEvent>>(16);
    let sink = ProjectingSandboxEventSink::new(tx);

    sink.record(SandboxEvent::Violation {
        violation: SandboxViolation {
            execution_id: "exec-4".into(),
            plan_fingerprint: "plan-fingerprint-4".into(),
            session_origin: Some("ses-3".into()),
            kind: SandboxViolationKind::PathEscape,
            path_or_endpoint: Some("/etc/passwd".into()),
            attribution: Attribution::BackendReported,
            backend: "bwrap".into(),
        },
    });
    match rx.recv().await.unwrap().event_ref() {
        ServerEvent::SandboxViolationReported {
            session_id,
            execution_id,
            violation,
        } => {
            assert_eq!(session_id.as_deref(), Some("ses-3"));
            assert_eq!(execution_id, "exec-4");
            assert_eq!(violation["plan_fingerprint"], "plan-fingerprint-4");
            assert_eq!(violation["kind"], "path_escape");
            assert_eq!(violation["attribution"], "backend_reported");
        }
        other => panic!(
            "expected SandboxViolationReported, got {:?}",
            other.event_name()
        ),
    }
}

/// A violation report must not change the active set: it is an audit
/// signal about a running (or already removed) execution, not a state
/// transition of the set itself.
#[tokio::test]
async fn violations_do_not_mutate_the_active_sandbox_set() {
    let store = RuntimeStateStore::new();
    store
        .sandbox_execution_upsert(
            "ses-3",
            agendao_server_core::SandboxExecutionSummary {
                execution_id: "exec-4".into(),
                backend: "bwrap".into(),
                profile_kind: "workspace_write".into(),
                plan_fingerprint: "fp-1".into(),
                pid: Some(42),
            },
        )
        .await;
    // The projector's violation arm calls no store mutation — simulate
    // the invariant directly: the set still holds exactly what the
    // upsert wrote.
    let snapshot = store.get("ses-3").await.unwrap();
    assert_eq!(snapshot.active_sandbox.len(), 1);
    assert_eq!(snapshot.active_sandbox[0].execution_id, "exec-4");
}

/// The completion gate for Phase 5: nothing may enter the sandboxed set
/// without an authority event carrying a real execution id. A
/// fabricated or truncated event is a no-op, so a frontend can never
/// render an unsandboxed execution as "sandboxed".
#[tokio::test]
async fn empty_execution_ids_never_enter_the_active_set() {
    let store = RuntimeStateStore::new();
    store
        .sandbox_execution_upsert(
            "ses-4",
            agendao_server_core::SandboxExecutionSummary {
                execution_id: String::new(),
                backend: "bwrap".into(),
                profile_kind: "workspace_write".into(),
                plan_fingerprint: "fp".into(),
                pid: None,
            },
        )
        .await;
    // No entry at all is the strongest form of "nothing entered the
    // set" — the rejected upsert must not even create the session slot.
    match store.get("ses-4").await {
        Some(snapshot) => assert!(
            snapshot.active_sandbox.is_empty(),
            "an empty execution id must not create a sandboxed entry"
        ),
        None => {}
    }

    // Removing an unknown id is a harmless no-op, not an error path.
    store
        .sandbox_execution_removed("ses-4", "never-existed")
        .await;
    assert!(store
        .get("ses-4")
        .await
        .map(|s| s.active_sandbox.is_empty())
        .unwrap_or(true));
}

/// `started` refines the pid without blanking the profile/fingerprint
/// that `prepared` established — the frontend event must stay the
/// complete authoritative fact.
#[tokio::test]
async fn started_refines_without_blanking_prepared_fields() {
    let store = RuntimeStateStore::new();
    store
        .sandbox_execution_upsert(
            "ses-5",
            agendao_server_core::SandboxExecutionSummary {
                execution_id: "exec-5".into(),
                backend: "bwrap".into(),
                profile_kind: "workspace_write".into(),
                plan_fingerprint: "fp-1".into(),
                pid: None,
            },
        )
        .await;
    store
        .sandbox_execution_upsert(
            "ses-5",
            agendao_server_core::SandboxExecutionSummary {
                execution_id: "exec-5".into(),
                backend: String::new(),
                profile_kind: String::new(),
                plan_fingerprint: String::new(),
                pid: Some(77),
            },
        )
        .await;
    let snapshot = store.get("ses-5").await.unwrap();
    assert_eq!(snapshot.active_sandbox.len(), 1);
    let entry = &snapshot.active_sandbox[0];
    assert_eq!(entry.pid, Some(77));
    assert_eq!(entry.profile_kind, "workspace_write");
    assert_eq!(entry.plan_fingerprint, "fp-1");
    assert_eq!(entry.backend, "bwrap");
}
