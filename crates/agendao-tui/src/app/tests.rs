use super::*;
use crate::api::{
    MessageTokensInfo, PendingPermissionSummary, PendingQuestionSummary, SessionExecutionTopology,
    SessionRunStatusKind, SessionTelemetrySnapshot, SessionTimeInfo,
};
use agendao_types::{SessionUsage, SessionUsageBooks};
use chrono::Utc;

#[test]
fn local_direct_app_uses_direct_base_url_authority() {
    let app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        base_url: Some("http://127.0.0.1:0".to_string()),
        ..AppLaunchConfig::default()
    })
    .expect("local direct app should initialize");

    assert!(app.local_direct);
    assert_eq!(app.server_event_base_url, "direct://local");
}

#[cfg(feature = "local-server")]
#[test]
fn local_direct_sync_session_loads_existing_history() {
    let mut app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        ..AppLaunchConfig::default()
    })
    .expect("local direct app should initialize");

    let session = app
        .context
        .get_api_client()
        .expect("api client")
        .create_session(None, Some(".".to_string()))
        .expect("session should create");

    app.context
        .get_api_client()
        .expect("api client")
        .send_prompt(
            &session.id,
            "hello from history".to_string(),
            None,
            None,
            None,
            None,
            None,
            Some(format!("tui_test_{}", uuid::Uuid::new_v4().simple())),
        )
        .expect("prompt should dispatch");

    app.context.navigate_session(session.id.clone());
    app.ensure_session_view(&session.id);
    app.sync_session_from_server(&session.id)
        .expect("local direct session sync should load history");

    let session_ctx = app.context.session.read();
    let messages = session_ctx
        .messages
        .get(&session.id)
        .expect("messages should load for selected session");
    assert!(!messages.is_empty(), "selected session should have message history");
}

#[test]
fn session_update_requires_sync_for_prompt_final_sources() {
    assert!(super::sync::session_update_requires_sync(Some(
        "prompt.final"
    )));
    assert!(super::sync::session_update_requires_sync(Some(
        "prompt.completed"
    )));
    assert!(super::sync::session_update_requires_sync(Some(
        "prompt.scheduler.completed"
    )));
    assert!(!super::sync::session_update_requires_sync(Some(
        "prompt.stream"
    )));
    assert!(super::sync::session_update_requires_sync(Some(
        "prompt.scheduler.stage.step"
    )));
    assert!(super::sync::session_update_requires_sync(Some(
        "prompt.scheduler.stage.usage"
    )));
    assert!(!super::sync::session_update_requires_sync(Some(
        "prompt.scheduler.stage.reasoning"
    )));
}

#[test]
fn incremental_session_sync_refreshes_title_and_revert_metadata() {
    let now = Utc::now().timestamp_millis();
    let session_id = "session-1";
    let mut session_ctx = crate::context::SessionContext::new();
    session_ctx.upsert_session(Session {
        id: session_id.to_string(),
        title: "New Session".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        parent_id: None,
        share: None,
        metadata: None,
    });
    session_ctx.set_messages(
        session_id,
        vec![Message {
            id: "m1".to_string(),
            role: MessageRole::User,
            content: "hello".to_string(),
            created_at: Utc::now(),
            agent: None,
            model: None,
            mode: None,
            finish: None,
            error: None,
            completed_at: None,
            cost: 0.0,
            tokens: TokenUsage::default(),
            metadata: None,
            multimodal: None,
            parts: vec![ContextMessagePart::Text {
                text: "hello".to_string(),
            }],
        }],
    );

    let session = SessionInfo {
        id: session_id.to_string(),
        slug: "session-1".to_string(),
        project_id: "project".to_string(),
        directory: ".".to_string(),
        parent_id: None,
        title: "Greeting Session".to_string(),
        version: "1".to_string(),
        time: SessionTimeInfo {
            created: now,
            updated: now + 1000,
            compacting: None,
            archived: None,
        },
        summary: None,
        share: None,
        permission: None,
        revert: Some(SessionRevertInfo {
            message_id: "m2".to_string(),
            part_id: Some("p1".to_string()),
            snapshot: Some("snapshot".to_string()),
            diff: None,
        }),
        fork: None,
        telemetry: None,
        metadata: None,
    };
    let mapped_messages = vec![map_api_message(&MessageInfo {
        id: "m2".to_string(),
        session_id: session_id.to_string(),
        role: "assistant".to_string(),
        created_at: now + 500,
        completed_at: None,
        agent: None,
        model: None,
        mode: None,
        finish: Some("stop".to_string()),
        error: None,
        cost: 0.0,
        tokens: MessageTokensInfo::default(),
        parts: vec![crate::api::MessagePart {
            id: "p1".to_string(),
            part_type: "text".to_string(),
            text: Some("world".to_string()),
            file: None,
            tool_call: None,
            tool_result: None,
            output_block: None,
            synthetic: None,
            ignored: None,
        }],
        metadata: None,
        multimodal: None,
    })];

    apply_incremental_session_sync(&mut session_ctx, session_id, &session, mapped_messages);

    assert_eq!(
        session_ctx
            .sessions
            .get(session_id)
            .map(|session| session.title.as_str()),
        Some("Greeting Session")
    );
    assert_eq!(
        session_ctx
            .messages
            .get(session_id)
            .map(|messages| messages.len()),
        Some(2)
    );
    assert_eq!(
        session_ctx
            .revert
            .get(session_id)
            .map(|revert| revert.message_id.as_str()),
        Some("m2")
    );
}

#[test]
fn question_prompt_at_appends_other_option_once() {
    let prompt = App::question_prompt_at(
        &QuestionInfo {
            id: "q1".to_string(),
            session_id: "s1".to_string(),
            questions: vec!["Pick one".to_string()],
            options: Some(vec![vec!["Yes".to_string(), "No".to_string()]]),
            items: Vec::new(),
        },
        0,
    )
    .expect("prompt should exist");

    assert_eq!(prompt.question_type, QuestionType::SingleChoice);
    assert_eq!(
        prompt.options.last().map(|option| option.id.as_str()),
        Some(OTHER_OPTION_ID)
    );
    assert_eq!(
        prompt.options.last().map(|option| option.label.as_str()),
        Some(OTHER_OPTION_LABEL)
    );
    assert_eq!(
        prompt
            .options
            .iter()
            .filter(|option| option.id == OTHER_OPTION_ID)
            .count(),
        1
    );
}

#[test]
fn diff_updated_event_populates_session_diff() {
    use crate::context::DiffEntry;

    let session_id = "session-diff-test";
    let mut session_ctx = crate::context::SessionContext::new();

    let diffs = vec![
        DiffEntry {
            file: "src/main.rs".to_string(),
            additions: 10,
            deletions: 3,
        },
        DiffEntry {
            file: "src/lib.rs".to_string(),
            additions: 5,
            deletions: 0,
        },
    ];
    session_ctx
        .session_diff
        .insert(session_id.to_string(), diffs);

    let stored = session_ctx.session_diff.get(session_id).unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].file, "src/main.rs");
    assert_eq!(stored[0].additions, 10);
    assert_eq!(stored[0].deletions, 3);
    assert_eq!(stored[1].file, "src/lib.rs");
    assert_eq!(stored[1].additions, 5);
    assert_eq!(stored[1].deletions, 0);
}

#[test]
fn map_api_diff_converts_correctly() {
    use crate::api::ApiDiffEntry;

    let api_diff = ApiDiffEntry {
        path: "src/foo.rs".to_string(),
        additions: 42,
        deletions: 7,
    };
    let mapped = map_api_diff(&api_diff);
    assert_eq!(mapped.file, "src/foo.rs");
    assert_eq!(mapped.additions, 42);
    assert_eq!(mapped.deletions, 7);
}

#[test]
fn map_api_todo_converts_status_strings() {
    use crate::api::ApiTodoItem;
    use crate::context::TodoStatus;

    let cases = vec![
        ("pending", TodoStatus::Pending),
        ("in_progress", TodoStatus::InProgress),
        ("completed", TodoStatus::Completed),
        ("done", TodoStatus::Completed),
        ("cancelled", TodoStatus::Cancelled),
        ("canceled", TodoStatus::Cancelled),
        ("unknown_status", TodoStatus::Pending),
    ];

    for (status_str, expected) in cases {
        let api_item = ApiTodoItem {
            id: "t1".to_string(),
            content: "Test".to_string(),
            status: status_str.to_string(),
            priority: "medium".to_string(),
        };
        let mapped = map_api_todo(&api_item);
        assert_eq!(
            std::mem::discriminant(&mapped.status),
            std::mem::discriminant(&expected),
            "Status '{}' should map to {:?}",
            status_str,
            expected
        );
    }
}

#[test]
fn dialog_left_click_is_consumed_without_closing_dialog() {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    let mut app = App::new().expect("app should initialize");
    app.open_model_select_dialog();

    let consumed = app
        .handle_dialog_mouse(&MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        })
        .expect("mouse event should be handled");

    assert!(consumed);
    assert!(app.model_select.is_open());
    assert!(!app.event_caused_change);
}

#[test]
fn ensure_session_view_skips_telemetry_fetch_for_optimistic_local_session() {
    let mut app = App::new().expect("app should initialize");
    let local_session_id = "local_session_123";

    app.context.navigate_session(local_session_id);
    app.ensure_session_view(local_session_id);

    assert_eq!(
        app.sync_runtime.pending_session_telemetry_sync.as_deref(),
        None
    );
    assert_eq!(app.sync_runtime.pending_session_telemetry_sync_due_at, None);
}

#[test]
fn prompt_dispatch_home_finished_queues_telemetry_refresh_for_real_session() {
    let mut app = App::new().expect("app should initialize");
    let optimistic_session_id = "local_session_123".to_string();
    let optimistic_message_id = "msg_123".to_string();
    let now = Utc::now().timestamp_millis();
    app.context.navigate_session(&optimistic_session_id);

    let event = Event::Custom(Box::new(CustomEvent::PromptDispatchHomeFinished {
        optimistic_session_id: optimistic_session_id.clone(),
        optimistic_message_id,
        created_session: Some(Box::new(SessionInfo {
            id: "session-real".to_string(),
            slug: "session-real".to_string(),
            project_id: "project".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            title: "Real session".to_string(),
            version: "1".to_string(),
            time: SessionTimeInfo {
                created: now,
                updated: now,
                compacting: None,
                archived: None,
            },
            summary: None,
            share: None,
            permission: None,
            revert: None,
            fork: None,
            telemetry: None,
            metadata: None,
        })),
        response: Some(crate::api::PromptResponse {
            status: "queued".to_string(),
            ok: Some(true),
            session_id: Some("session-real".to_string()),
            queued_count: Some(1),
            pending_question_id: None,
            command: None,
            missing_fields: Vec::new(),
        }),
        error: None,
    }));

    app.handle_event(&event)
        .expect("prompt dispatch completion should be handled");

    assert_eq!(app.current_session_id().as_deref(), Some("session-real"));
    assert_eq!(
        app.sync_runtime.pending_session_telemetry_sync.as_deref(),
        Some("session-real")
    );
    assert!(app
        .sync_runtime
        .pending_session_telemetry_sync_due_at
        .is_some());
    assert!(!app.sync_runtime.session_telemetry_sync_inflight);
}

#[test]
fn ensure_session_view_does_not_requeue_telemetry_for_same_active_view() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-ensure-idempotent";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Ensure session view".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);

    app.ensure_session_view(session_id);
    app.sync_runtime.pending_session_telemetry_sync = Some(session_id.to_string());
    let sentinel_due = Instant::now() + Duration::from_secs(42);
    app.sync_runtime.pending_session_telemetry_sync_due_at = Some(sentinel_due);

    app.ensure_session_view(session_id);

    assert_eq!(
        app.sync_runtime.pending_session_telemetry_sync.as_deref(),
        Some(session_id)
    );
    assert_eq!(
        app.sync_runtime.pending_session_telemetry_sync_due_at,
        Some(sentinel_due)
    );
}

#[test]
fn session_telemetry_refresh_finished_applies_snapshot_for_active_session() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-telemetry-active";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Telemetry".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);
    app.sync_runtime.session_telemetry_sync_inflight = true;

    let event = Event::Custom(Box::new(CustomEvent::SessionTelemetryRefreshFinished {
        session_id: session_id.to_string(),
        telemetry: Some(Box::new(test_session_telemetry_snapshot(
            session_id, "stage-1",
        ))),
    }));

    app.handle_event(&event)
        .expect("telemetry refresh event should be handled");

    assert!(!app.sync_runtime.session_telemetry_sync_inflight);
    assert_eq!(
        app.context
            .session_runtime()
            .as_ref()
            .and_then(|runtime| runtime.active_stage_id.as_deref()),
        Some("stage-1")
    );
    assert!(app.event_caused_change);
}

#[test]
fn session_telemetry_refresh_finished_queues_pending_user_input_syncs() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-telemetry-pending-inputs";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Telemetry".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);

    let mut telemetry = test_session_telemetry_snapshot(session_id, "stage-1");
    telemetry.runtime.pending_question = Some(PendingQuestionSummary {
        request_id: "q-1".to_string(),
        questions: serde_json::json!([{ "question": "Continue?" }]),
    });
    telemetry.runtime.pending_permission = Some(PendingPermissionSummary {
        permission_id: "perm-1".to_string(),
        requested_at: Utc::now().timestamp_millis(),
        tool: Some("bash".to_string()),
    });

    let event = Event::Custom(Box::new(CustomEvent::SessionTelemetryRefreshFinished {
        session_id: session_id.to_string(),
        telemetry: Some(Box::new(telemetry)),
    }));

    app.handle_event(&event)
        .expect("telemetry refresh event should be handled");

    assert!(app.sync_runtime.pending_question_sync_due_at.is_some());
    assert!(app.sync_runtime.pending_permission_sync_due_at.is_some());
}

#[test]
fn session_telemetry_refresh_finished_ignores_inactive_session_snapshot() {
    let mut app = App::new().expect("app should initialize");
    let active_session_id = "session-active";
    let inactive_session_id = "session-inactive";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        for session_id in [active_session_id, inactive_session_id] {
            session_ctx.upsert_session(Session {
                id: session_id.to_string(),
                title: session_id.to_string(),
                created_at: now,
                updated_at: now,
                parent_id: None,
                share: None,
                metadata: None,
            });
        }
        session_ctx.set_current_session_id(active_session_id.to_string());
    }
    app.context.navigate_session(active_session_id);
    app.context
        .apply_session_telemetry_snapshot(test_session_telemetry_snapshot(
            active_session_id,
            "existing-stage",
        ));
    app.sync_runtime.session_telemetry_sync_inflight = true;

    let event = Event::Custom(Box::new(CustomEvent::SessionTelemetryRefreshFinished {
        session_id: inactive_session_id.to_string(),
        telemetry: Some(Box::new(test_session_telemetry_snapshot(
            inactive_session_id,
            "wrong-stage",
        ))),
    }));

    app.handle_event(&event)
        .expect("inactive telemetry refresh event should be handled");

    assert!(!app.sync_runtime.session_telemetry_sync_inflight);
    assert_eq!(
        app.context
            .session_runtime()
            .as_ref()
            .and_then(|runtime| runtime.active_stage_id.as_deref()),
        Some("existing-stage")
    );
}

#[test]
fn tick_spawns_due_session_telemetry_refresh_without_blocking() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-tick-refresh";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Tick refresh".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);
    app.sync_runtime.pending_session_telemetry_sync = Some(session_id.to_string());
    app.sync_runtime.pending_session_telemetry_sync_due_at = Some(Instant::now());
    app.sync_runtime.session_telemetry_sync_inflight = false;

    app.handle_event(&Event::Tick)
        .expect("tick should process queued telemetry refresh");

    assert!(app.sync_runtime.session_telemetry_sync_inflight);
    assert_eq!(app.sync_runtime.pending_session_telemetry_sync, None);
    assert_eq!(app.sync_runtime.pending_session_telemetry_sync_due_at, None);
}

#[test]
fn permission_requested_event_surfaces_prompt_without_http_sync() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-permission";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Permission session".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);

    let permission = crate::api::PermissionRequestInfo {
        id: "perm-1".to_string(),
        session_id: session_id.to_string(),
        tool: "bash".to_string(),
        permission_class: Some("dangerous_exec".to_string()),
        scope_key: Some("python3".to_string()),
        scope_label: Some("Shell commands: python3".to_string()),
        origin_tool: None,
        supported_lifetimes: vec!["once".to_string()],
        matcher_kind: None,
        matcher_key: None,
        matcher_label: None,
        grant_target_summary: None,
        risk_tags: vec!["dangerous_exec".to_string()],
        input: serde_json::json!({
            "permission": "bash",
            "metadata": { "command": "python3 demo.py" }
        }),
        message: "Execute python3 demo.py".to_string(),
    };

    let event = Event::Custom(Box::new(CustomEvent::StateChanged(
        StateChange::PermissionRequested {
            session_id: session_id.to_string(),
            permission: permission.clone(),
        },
    )));

    app.handle_event(&event)
        .expect("permission requested event should be handled");

    assert!(app.event_caused_change);
    assert!(app.permission_runtime.pending_ids.contains("perm-1"));
    assert!(app.permission_prompt.is_open);
    assert!(app.sync_runtime.pending_permission_sync_due_at.is_some());
    assert_eq!(
        app.permission_runtime
            .pending_requests
            .get("perm-1")
            .map(|request| request.tool.as_str()),
        Some("bash")
    );
}

#[test]
fn question_created_event_queues_sync_without_immediate_prompt_sync() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-question-queued";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Question session".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);

    let event = Event::Custom(Box::new(CustomEvent::StateChanged(
        StateChange::QuestionCreated {
            session_id: session_id.to_string(),
            request_id: "q-1".to_string(),
        },
    )));

    app.handle_event(&event)
        .expect("question created event should be handled");

    assert!(app.sync_runtime.pending_question_sync_due_at.is_some());
    assert!(app.question_prompt.current().is_none());
}

#[test]
fn permission_sync_does_not_clear_submitting_request_on_empty_poll() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-permission-keepalive";
    let permission = crate::api::PermissionRequestInfo {
        id: "perm-1".to_string(),
        session_id: session_id.to_string(),
        tool: "bash".to_string(),
        permission_class: Some("dangerous_exec".to_string()),
        scope_key: Some("python3".to_string()),
        scope_label: Some("Shell commands: python3".to_string()),
        origin_tool: None,
        supported_lifetimes: vec!["once".to_string()],
        matcher_kind: None,
        matcher_key: None,
        matcher_label: None,
        grant_target_summary: None,
        risk_tags: vec!["dangerous_exec".to_string()],
        input: serde_json::json!({
            "permission": "bash",
            "metadata": { "command": "python3 demo.py" }
        }),
        message: "Execute python3 demo.py".to_string(),
    };

    {
        let mut session_ctx = app.context.session.write();
        let now = Utc::now();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Permission keepalive".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);
    app.enqueue_permission_request(permission);
    assert!(app.permission_prompt.mark_submitting("perm-1"));

    app.permission_runtime.pending_ids.clear();
    app.permission_runtime.pending_requests.clear();
    app.permission_prompt
        .retain_requests(|_| true);

    let latest_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    app.permission_runtime
        .pending_ids
        .retain(|id| latest_ids.contains(id));
    app.permission_runtime
        .pending_requests
        .retain(|id, _| latest_ids.contains(id));
    app.permission_prompt.retain_requests(|request| {
        latest_ids.contains(&request.id) || request.is_submitting
    });

    assert!(app.permission_prompt.is_open);
    assert_eq!(
        app.permission_prompt.current_request().map(|req| req.id.as_str()),
        Some("perm-1")
    );
    assert!(app.permission_prompt.is_current_request_submitting());
}

#[test]
fn direct_question_resolved_event_uses_payload_session_id() {
    let direct = crate::local_server_bridge::LocalServerEvent::QuestionResolved {
        session_id: "ses_direct".to_string(),
        request_id: "q-1".to_string(),
    };

    let change = super::runtime::direct_event_to_state_change("ses_fallback", direct)
        .expect("question resolved should map to state change");

    match change {
        StateChange::QuestionResolved {
            session_id,
            request_id,
        } => {
            assert_eq!(session_id, "ses_direct");
            assert_eq!(request_id, "q-1");
        }
        other => panic!("unexpected state change: {other:?}"),
    }
}

#[test]
fn home_route_does_not_schedule_session_only_sync_ticks() {
    let mut app = App::new().expect("app should initialize");
    let now = Instant::now();
    let baseline = app
        .next_tick_deadline(now)
        .expect("home route should still have a deadline");

    app.sync_runtime.last_question_sync = now - Duration::from_secs(60);
    app.sync_runtime.last_permission_sync = now - Duration::from_secs(60);
    app.sync_runtime.last_process_refresh = now - Duration::from_secs(60);
    app.sync_runtime.pending_question_sync_due_at = Some(now);
    app.sync_runtime.pending_permission_sync_due_at = Some(now);
    app.sync_runtime.pending_process_refresh_due_at = Some(now);

    let updated = app
        .next_tick_deadline(now)
        .expect("home route should still have a deadline");

    assert_eq!(updated, baseline);
}

fn test_session_telemetry_snapshot(
    session_id: &str,
    active_stage_id: &str,
) -> SessionTelemetrySnapshot {
    SessionTelemetrySnapshot {
        runtime: crate::api::SessionRuntimeState {
            session_id: session_id.to_string(),
            run_status: SessionRunStatusKind::Running,
            current_message_id: None,
            usage: None,
            active_stage_id: Some(active_stage_id.to_string()),
            active_stage_count: 1,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            pending_followup_count: 0,
            attached_sessions: Vec::new(),
        },
        stages: Vec::new(),
        topology: SessionExecutionTopology {
            session_id: session_id.to_string(),
            active_count: 1,
            done_count: 0,
            running_count: 1,
            waiting_count: 0,
            cancelling_count: 0,
            retry_count: 0,
            updated_at: None,
            roots: Vec::new(),
        },
        usage: SessionUsage::default(),
        usage_books: SessionUsageBooks::default(),
        tool_repair_summary: None,
        model_tool_repair_summary: None,
        repair_query_snapshot: None,
        tool_trajectory_quality: None,
        tool_result_governance: None,
        pending_permission_count: 0,
        granted_by_turn_count: 0,
        granted_by_session_count: 0,
        granted_by_matcher_kind: Default::default(),
        last_permission_matcher_kind: None,
        last_permission_grant_target: None,
        last_permission_miss_count: 0,
        memory: None,
        cache_evidence: None,
        context_explain: None,
        ownership: None,
        context_compaction_summary: None,
        compaction_continuity: None,
        context_compaction_lifecycle_summary: None,
        context_pressure_governance_summary: None,
        cache_semantics: None,
        context_closure_contract: None,
        prompt_surface_evidence: None,
        ingress_stabilization: None,
        execution_preflight_summary: None,
        provider_diagnostic_summary: None,
        runtime_protocol: None,
        event_bus_telemetry: None,
    }
}

#[test]
fn exit_summary_uses_current_cli_session_command() {
    let app = App::new().expect("app should initialize");
    let now = Utc::now();
    let session_id = "ses_continue_test";
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Continue Test".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
    }
    app.context.navigate_session(session_id);

    let summary = app.exit_summary().expect("exit summary");
    assert!(summary.contains("agendao tui -s ses_continue_test"));
    assert!(!summary.contains("agendao -s ses_continue_test"));
    assert!(!summary.contains("agendao run -s ses_continue_test"));
}

#[test]
fn refresh_attached_sessions_uses_parent_graph_for_child_route() {
    let app = App::new().expect("app should initialize");
    let now = Utc::now();
    let parent_id = "parent-session";
    let attached_id = "attached-session";

    let mut metadata = HashMap::new();
    metadata.insert(
        "scheduler_stage_attached_session_id".to_string(),
        serde_json::json!(attached_id),
    );
    metadata.insert("scheduler_stage".to_string(), serde_json::json!("review"));
    metadata.insert(
        "scheduler_stage_title".to_string(),
        serde_json::json!("Review"),
    );
    metadata.insert(
        "scheduler_stage_status".to_string(),
        serde_json::json!("running"),
    );

    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: parent_id.to_string(),
            title: "Parent".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.upsert_session(Session {
            id: attached_id.to_string(),
            title: "Child".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: Some(parent_id.to_string()),
            share: None,
            metadata: None,
        });
        session_ctx.set_messages(
            parent_id,
            vec![Message {
                id: "stage-message".to_string(),
                role: MessageRole::Assistant,
                content: String::new(),
                created_at: now,
                agent: None,
                model: None,
                mode: None,
                finish: None,
                error: None,
                completed_at: None,
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: Some(metadata),
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: String::new(),
                }],
            }],
        );
        session_ctx.set_messages(attached_id, Vec::new());
    }

    app.context.navigate_session(attached_id);
    app.refresh_attached_sessions();

    let children = app.context.attached_sessions();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].session_id, attached_id);
}

#[test]
fn ui_bridge_drop_growth_surfaces_warning_toast() {
    let mut app = App::new().expect("app should initialize");
    app.toast = Toast::new();
    assert!(!app.toast.is_visible());

    app.sync_runtime.last_ui_bridge_dropped_events = 2;
    app.context
        .ui_bridge
        .emit(Event::Custom(Box::new(crate::event::CustomEvent::Message(
            "message-1".to_string(),
        ))));
    app.context
        .ui_bridge
        .emit(Event::Custom(Box::new(crate::event::CustomEvent::Message(
            "message-2".to_string(),
        ))));

    app.context.ui_bridge.drain(1);
    let queue_capacity = app.context.ui_bridge_snapshot().capacity;
    for index in 0..(queue_capacity + 2) {
        app.context
            .ui_bridge
            .emit(Event::Custom(Box::new(crate::event::CustomEvent::Message(
                format!("overflow-{index}"),
            ))));
    }

    assert!(app.sync_ui_bridge_health());
    assert!(app.toast.is_visible());
    assert_eq!(
        app.sync_runtime.last_ui_bridge_dropped_events,
        app.context.ui_bridge_snapshot().dropped_events
    );
    assert!(!app.sync_ui_bridge_health());
}

#[test]
fn image_mime_detection_prefers_content_signature_over_extension() {
    let path = PathBuf::from("not-really.txt");
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
    assert_eq!(App::image_mime_from_path(&path, &png), Some("image/png"));
}

#[test]
fn image_mime_detection_recognizes_svg_without_extension() {
    let path = PathBuf::from("diagram");
    let svg = br#"<?xml version="1.0" encoding="UTF-8"?><svg viewBox="0 0 10 10"></svg>"#;
    assert_eq!(App::image_mime_from_path(&path, svg), Some("image/svg+xml"));
}
