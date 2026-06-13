use super::*;
use crate::api::{
    MessageTokensInfo, PendingPermissionSummary, PendingQuestionSummary, SessionExecutionTopology,
    SessionRunStatusKind, SessionTelemetrySnapshot, SessionTimeInfo,
};
use agendao_server_core::frontend_events::FrontendEvent;
use agendao_stage_protocol::{StageStatus, StageSummary};
use agendao_types::{SessionUsage, SessionUsageBooks};
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::state::DialogSlot;

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

#[cfg(feature = "local-server")]
#[test]
fn local_direct_home_submit_uses_real_session_id_immediately() {
    let root =
        std::env::temp_dir().join(format!("agendao-tui-home-submit-{}", uuid::Uuid::new_v4()));
    let workspace_root = root.join("workspace");
    let data_root = root.join("data");
    std::fs::create_dir_all(&workspace_root).expect("create temp workspace root");
    std::fs::create_dir_all(&data_root).expect("create temp data root");
    unsafe {
        std::env::set_var("AGENDAO_DATA_DIR", &data_root);
        std::env::set_var("XDG_DATA_HOME", &data_root);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let local_server = runtime
        .block_on(crate::local_server_bridge::new_local_server_for_workspace(
            workspace_root,
        ))
        .expect("local server");
    let mut app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        local_server: Some(local_server),
        ..AppLaunchConfig::default()
    })
    .expect("local direct app should initialize");

    app.prompt.set_input("hello live output".to_string());
    app.submit_prompt().expect("prompt should submit");

    let current_session_id = app.current_session_id().expect("current session");
    assert!(
        !current_session_id.starts_with("local_session_"),
        "direct home submit must switch to a real session before dispatch, got {current_session_id}"
    );
    assert_eq!(
        app.sse_session_filter.borrow().as_deref(),
        Some(current_session_id.as_str())
    );
}

#[test]
fn prompt_edited_event_updates_app_prompt_state() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("edited in component".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
        prompt: Box::new(prompt),
    })))
    .expect("prompt edited event should apply");

    assert_eq!(app.prompt.get_input(), "edited in component");
}

#[test]
fn prompt_edited_event_opens_slash_popup_for_slash_commands() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("/help".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
        prompt: Box::new(prompt),
    })))
    .expect("prompt edited event should apply");

    assert!(app.context.has_open_dialogs());
}

#[test]
fn prompt_submit_requested_event_reuses_existing_submit_flow() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("submit through component".to_string());

    app.process_event(&Event::Custom(Box::new(
        CustomEvent::PromptSubmitRequested {
            prompt: Box::new(prompt),
        },
    )))
    .expect("prompt submit requested event should submit");

    assert!(app.prompt.get_input().is_empty());
}

#[test]
fn prompt_submit_requested_from_home_navigates_to_session_route() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("Hi".to_string());

    app.process_event(&Event::Custom(Box::new(
        CustomEvent::PromptSubmitRequested {
            prompt: Box::new(prompt),
        },
    )))
    .expect("prompt submit requested event should submit");

    assert!(
        matches!(app.context.current_route(), Route::Session { .. }),
        "home submit should navigate to session route"
    );
}

#[test]
fn prompt_submit_requested_help_command_opens_help_dialog() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("/help".to_string());

    app.process_event(&Event::Custom(Box::new(
        CustomEvent::PromptSubmitRequested {
            prompt: Box::new(prompt),
        },
    )))
    .expect("prompt submit requested event should execute help command");

    assert!(app.context.has_open_dialogs());
}

#[test]
fn slash_popup_enter_executes_selected_help_action() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("/help".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
        prompt: Box::new(prompt),
    })))
    .expect("prompt edited event should open slash popup");

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))
    .expect("enter should execute slash popup selection");

    assert!(app.context.is_dialog_open(DialogSlot::Help));
}

#[test]
fn slash_popup_intent_close_closes_popup() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("/help".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
        prompt: Box::new(prompt),
    })))
    .expect("prompt edited event should open slash popup");

    assert!(app.context.is_dialog_open(DialogSlot::SlashPopup));

    app.process_event(&Event::Custom(Box::new(CustomEvent::SlashPopupIntent {
        kind: crate::event::SlashPopupIntentKind::Close,
    })))
    .expect("slash popup close intent should process");

    assert!(!app.context.is_dialog_open(DialogSlot::SlashPopup));
}

#[test]
fn slash_popup_intent_select_current_executes_selected_action() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("/help".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
        prompt: Box::new(prompt),
    })))
    .expect("prompt edited event should open slash popup");

    app.process_event(&Event::Custom(Box::new(CustomEvent::SlashPopupIntent {
        kind: crate::event::SlashPopupIntentKind::SelectCurrent,
    })))
    .expect("slash popup select intent should process");

    assert!(app.context.is_dialog_open(DialogSlot::Help));
}

#[test]
fn reactive_route_slash_popup_char_input_falls_through_to_prompt_authority() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("/he".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
        prompt: Box::new(prompt),
    })))
    .expect("prompt edited event should open slash popup");

    assert!(app.context.is_dialog_open(DialogSlot::SlashPopup));

    let consumed = app
        .handle_dialog_key(KeyEvent::new(
        KeyCode::Char('l'),
        KeyModifiers::NONE,
    ))
        .expect("reactive slash popup should yield text input back to prompt");

    assert!(
        !consumed,
        "reactive slash popup text keys must fall through to reratui prompt authority"
    );
    assert_eq!(app.prompt.get_input(), "/he");
    assert_eq!(app.slash_popup.query(), "he");
}

#[test]
fn reactive_route_slash_popup_backspace_falls_through_to_prompt_authority() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("/help".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
        prompt: Box::new(prompt),
    })))
    .expect("prompt edited event should open slash popup");

    assert!(app.context.is_dialog_open(DialogSlot::SlashPopup));

    let consumed = app
        .handle_dialog_key(KeyEvent::new(
        KeyCode::Backspace,
        KeyModifiers::NONE,
    ))
        .expect("reactive slash popup should yield backspace to prompt");

    assert!(
        !consumed,
        "reactive slash popup backspace must fall through to reratui prompt authority"
    );
    assert_eq!(app.prompt.get_input(), "/help");
    assert_eq!(app.slash_popup.query(), "help");
}

#[test]
fn reactive_route_slash_popup_space_falls_through_to_prompt_authority() {
    let mut app = App::new().expect("app should initialize");
    let mut prompt = app.prompt.clone();
    prompt.set_input("/model".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
        prompt: Box::new(prompt),
    })))
    .expect("prompt edited event should open slash popup");

    assert!(app.context.is_dialog_open(DialogSlot::SlashPopup));

    let consumed = app
        .handle_dialog_key(KeyEvent::new(
        KeyCode::Char(' '),
        KeyModifiers::NONE,
    ))
        .expect("reactive slash popup should yield space to prompt");

    assert!(
        !consumed,
        "reactive slash popup space must fall through to reratui prompt authority"
    );
    assert_eq!(app.prompt.get_input(), "/model");
    assert_eq!(app.slash_popup.query(), "model");
}

#[test]
fn prompt_paste_text_event_updates_app_prompt_state() {
    let mut app = App::new().expect("app should initialize");

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptPasteText {
        text: "pasted text".to_string(),
    })))
    .expect("prompt paste text event should apply");

    assert_eq!(app.prompt.get_input(), "pasted text");
}

#[test]
fn ui_action_requested_help_opens_help_dialog() {
    let mut app = App::new().expect("app should initialize");

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::ShowHelp,
    })))
    .expect("ui action request should execute");

    assert!(app.context.is_dialog_open(DialogSlot::Help));
}

#[test]
fn ui_action_requested_clear_prompt_discards_draft() {
    let mut app = App::new().expect("app should initialize");
    app.prompt.set_input("to be cleared".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::ClearPrompt,
    })))
    .expect("ui action request should execute");

    assert!(app.prompt.get_input().is_empty());
}

#[test]
fn ui_action_requested_model_opens_model_dialog() {
    let mut app = App::new().expect("app should initialize");

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::OpenModelList,
    })))
    .expect("ui action request should execute");

    assert!(app.context.is_dialog_open(DialogSlot::ModelSelect));
}

#[test]
fn ui_action_requested_variant_cycles_model_variant() {
    let mut app = App::new().expect("app should initialize");
    app.available_models = ["model-a".to_string(), "model-a/fast".to_string()]
        .into_iter()
        .collect();
    app.model_variants.insert(
        "model-a".to_string(),
        vec!["fast".to_string()],
    );
    app.context.set_model_selection("model-a".to_string(), None);
    app.context.set_model_variant(None);

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::CycleVariant,
    })))
    .expect("ui action request should execute");

    assert_eq!(app.context.current_model_variant().as_deref(), Some("fast"));
}

#[test]
fn ui_action_requested_toggle_thinking_updates_preferences() {
    let mut app = App::new().expect("app should initialize");
    let before = app.context.show_thinking_enabled();

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::ToggleThinking,
    })))
    .expect("ui action request should execute");

    assert_ne!(app.context.show_thinking_enabled(), before);
}

#[test]
fn ui_action_requested_toggle_tool_details_updates_preferences() {
    let mut app = App::new().expect("app should initialize");
    let before = app.context.show_tool_details_enabled();

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::ToggleToolDetails,
    })))
    .expect("ui action request should execute");

    assert_ne!(app.context.show_tool_details_enabled(), before);
}

#[test]
fn ui_action_requested_cycle_agent_next_rotates_current_agent() {
    let mut app = App::new().expect("app should initialize");
    let first = app
        .agent_select
        .agents()
        .first()
        .expect("default agent")
        .name
        .clone();
    let second = app
        .agent_select
        .agents()
        .get(1)
        .expect("second default agent")
        .name
        .clone();
    app.context.set_agent(first);

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::CycleAgentNext,
    })))
    .expect("ui action request should execute");

    assert_eq!(app.context.current_agent(), second);
}

#[test]
fn ui_action_requested_cycle_agent_previous_rotates_current_agent_backward() {
    let mut app = App::new().expect("app should initialize");
    let first = app
        .agent_select
        .agents()
        .first()
        .expect("default agent")
        .name
        .clone();
    let last = app
        .agent_select
        .agents()
        .last()
        .expect("last default agent")
        .name
        .clone();
    app.context.set_agent(first);

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::CycleAgentPrevious,
    })))
    .expect("ui action request should execute");

    assert_eq!(app.context.current_agent(), last);
}

#[test]
fn ui_action_requested_paste_clipboard_executes_without_error() {
    let mut app = App::new().expect("app should initialize");

    app.process_event(&Event::Custom(Box::new(CustomEvent::PromptPasteText {
        text: "clipboard text".to_string(),
    })))
    .expect("seed prompt paste text");

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::PasteClipboard,
    })))
    .expect("ui action request should execute");
}

#[test]
fn ui_action_requested_cut_prompt_clears_input() {
    let mut app = App::new().expect("app should initialize");
    app.prompt.set_input("cut me".to_string());

    app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
        action: agendao_command::UiActionId::CutPrompt,
    })))
    .expect("ui action request should execute");

    assert!(app.prompt.get_input().is_empty());
}

#[test]
fn reactive_route_ctrl_k_global_handler_yields_to_prompt_authority() {
    let mut app = App::new().expect("app should initialize");

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('k'),
        KeyModifiers::CONTROL,
    )))
    .expect("reactive route ctrl-k should no-op in global handler");

    let drained = app.context.drain_ui_events(4);
    assert!(
        drained.is_empty(),
        "global handler must not emit or execute abort directly under reactive prompt authority"
    );
}

#[test]
fn reactive_route_escape_global_handler_yields_to_prompt_authority() {
    let mut app = App::new().expect("app should initialize");

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )))
    .expect("reactive route escape should no-op in global handler when no selection is active");

    let drained = app.context.drain_ui_events(4);
    assert!(
        drained.is_empty(),
        "global handler must not execute session interrupt directly under reactive prompt authority"
    );
}

#[test]
fn reactive_route_ctrl_c_global_handler_yields_to_prompt_authority_without_selection() {
    let mut app = App::new().expect("app should initialize");

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )))
    .expect("reactive route ctrl-c should no-op in global handler without selection");

    assert!(!app.is_exiting());
    let drained = app.context.drain_ui_events(4);
    assert!(
        drained.is_empty(),
        "global handler must not execute exit directly under reactive prompt authority"
    );
}

#[test]
fn reactive_route_ctrl_shift_c_global_handler_yields_to_prompt_authority_without_selection() {
    let mut app = App::new().expect("app should initialize");

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    )))
    .expect("reactive route ctrl+shift+c should no-op in global handler without selection");

    let drained = app.context.drain_ui_events(4);
    assert!(
        drained.is_empty(),
        "global handler must not execute copy-selection directly under reactive prompt authority"
    );
}

#[test]
fn session_interrupt_requested_exits_shell_mode() {
    let mut app = App::new().expect("app should initialize");
    app.prompt.handle_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
    assert!(app.prompt.is_shell_mode(), "prompt should enter shell mode");

    app.process_event(&Event::Custom(Box::new(
        CustomEvent::SessionInterruptRequested,
    )))
    .expect("session interrupt request should process");

    assert!(
        !app.prompt.is_shell_mode(),
        "interrupt request should exit shell mode"
    );
}

#[test]
fn session_interrupt_requested_requires_confirmation_before_interrupting_running_session() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-interrupt-confirm";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Interrupt confirm".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::Running);
    }
    app.context.navigate_session(session_id);

    app.process_event(&Event::Custom(Box::new(
        CustomEvent::SessionInterruptRequested,
    )))
    .expect("first interrupt request should process");

    let status = {
        let session_ctx = app.context.session.read();
        session_ctx.status(session_id).clone()
    };
    assert!(
        matches!(status, SessionStatus::Running),
        "first interrupt request should not abort immediately, got {status:?}"
    );
}

#[test]
fn session_interrupt_requested_second_press_sets_running_session_idle() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-interrupt-confirm-2";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Interrupt confirm second".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::Running);
    }
    app.context.navigate_session(session_id);

    app.process_event(&Event::Custom(Box::new(
        CustomEvent::SessionInterruptRequested,
    )))
    .expect("first interrupt request should process");
    app.process_event(&Event::Custom(Box::new(
        CustomEvent::SessionInterruptRequested,
    )))
    .expect("second interrupt request should process");

    let status = {
        let session_ctx = app.context.session.read();
        session_ctx.status(session_id).clone()
    };
    assert!(
        matches!(status, SessionStatus::Idle),
        "second interrupt request should settle to idle, got {status:?}"
    );
}

#[test]
fn raw_paste_event_targets_provider_dialog_not_prompt() {
    let mut app = App::new().expect("app should initialize");
    app.prompt.set_input("prompt".to_string());
    app.open_provider_dialog_modal();
    let provider = crate::render::Provider {
        id: "demo".to_string(),
        name: "Demo".to_string(),
        env_hint: "DEMO_API_KEY".to_string(),
        base_url: None,
        protocol: None,
        descriptor_candidate: None,
        descriptor_candidate_error: None,
        model_count: 0,
        status: crate::render::ProviderStatus::Disconnected,
    };
    app.provider_dialog.enter_input_mode_for_provider(provider);

    app.process_event(&Event::Paste("secret".to_string()))
        .expect("paste event should be handled");

    assert_eq!(app.prompt.get_input(), "prompt");
    let pending = app.provider_dialog.pending_submit();
    match pending {
        Some(crate::render::PendingSubmit::Known {
            provider_id,
            api_key,
        }) => {
            assert_eq!(provider_id, "demo");
            assert_eq!(api_key, "secret");
        }
        other => panic!("unexpected pending submit: {:?}", other),
    }
}

#[test]
fn ensure_session_view_after_sync_preserves_loaded_history() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-loaded-before-view";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Loaded session".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_messages(
            session_id,
            vec![Message {
                id: "m1".to_string(),
                role: MessageRole::Assistant,
                content: "hello".to_string(),
                created_at: now,
                agent: None,
                model: None,
                mode: None,
                finish: Some("stop".to_string()),
                error: None,
                completed_at: Some(now),
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: "hello".to_string(),
                }],
            }],
        );
    }
    app.context.navigate_session(session_id);

    app.ensure_session_view(session_id);

    let messages = app
        .context
        .session
        .read()
        .messages
        .get(session_id)
        .cloned()
        .expect("messages should remain loaded");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "m1");
}

#[test]
fn plain_q_does_not_exit_tui() {
    let mut app = App::new().expect("app should initialize");

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::NONE,
    )))
    .expect("plain q should be handled");

    assert!(!app.is_exiting());
}

#[test]
fn ctrl_x_then_q_exits_tui() {
    let mut app = App::new().expect("app should initialize");

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::CONTROL,
    )))
    .expect("ctrl+x should arm leader state");
    assert!(app.leader_state.active);

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::NONE,
    )))
    .expect("leader q should exit");

    assert!(app.is_exiting());
}

#[test]
fn ctrl_c_without_selection_exits_via_ui_action() {
    let mut app = App::new().expect("app should initialize");

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )))
    .expect("ctrl+c should be handled");

    assert!(app.is_exiting());
}

#[test]
fn ctrl_c_with_selection_copies_instead_of_exiting() {
    let mut app = App::new().expect("app should initialize");
    app.selection.start(1, 1);
    app.selection.finalize();

    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )))
    .expect("ctrl+c should be handled");

    assert!(!app.is_exiting());
}

#[test]
fn navigate_session_with_prompt_cleanup_clears_selection() {
    let mut app = App::new().expect("app should initialize");
    app.selection.start(3, 5);
    app.selection.finalize();

    app.navigate_session_with_prompt_cleanup("session-target".to_string());

    assert!(!app.selection.is_active());
}

#[test]
fn session_list_dialog_can_switch_between_existing_sessions() {
    let mut app = App::new().expect("app should initialize");
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: "session-1".to_string(),
            title: "First".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.upsert_session(Session {
            id: "session-2".to_string(),
            title: "Second".to_string(),
            created_at: now,
            updated_at: now - chrono::Duration::seconds(1),
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_messages(
            "session-1",
            vec![Message {
                id: "m1".to_string(),
                role: MessageRole::Assistant,
                content: "one".to_string(),
                created_at: now,
                agent: None,
                model: None,
                mode: None,
                finish: Some("stop".to_string()),
                error: None,
                completed_at: Some(now),
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: "one".to_string(),
                }],
            }],
        );
        session_ctx.set_messages(
            "session-2",
            vec![Message {
                id: "m2".to_string(),
                role: MessageRole::Assistant,
                content: "two".to_string(),
                created_at: now,
                agent: None,
                model: None,
                mode: None,
                finish: Some("stop".to_string()),
                error: None,
                completed_at: Some(now),
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: "two".to_string(),
                }],
            }],
        );
    }
    app.context.navigate_session("session-1");
    app.ensure_session_view("session-1");
    app.session_list_dialog.set_sessions(vec![
        SessionItem {
            id: "session-1".to_string(),
            title: "First".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            updated_at: now.timestamp_millis(),
            is_busy: false,
        },
        SessionItem {
            id: "session-2".to_string(),
            title: "Second".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            updated_at: (now - chrono::Duration::seconds(1)).timestamp_millis(),
            is_busy: false,
        },
    ]);
    app.open_session_list_dialog_modal(Some("session-1"));
    app.session_list_dialog.move_down();

    app.handle_dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("session list enter should navigate");

    assert_eq!(app.current_session_id().as_deref(), Some("session-2"));
    assert_eq!(
        app.context
            .session_view_handle()
            .as_ref()
            .map(|view| view.session_id()),
        Some("session-2")
    );
    let messages = app
        .context
        .session
        .read()
        .messages
        .get("session-2")
        .cloned()
        .expect("second session messages should remain available");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "two");
}

#[test]
fn prompt_submit_requested_session_command_opens_session_list_from_session_route() {
    let mut app = App::new().expect("app should initialize");
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: "session-1".to_string(),
            title: "First".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id("session-1".to_string());
    }
    app.context.navigate_session("session-1");
    app.ensure_session_view("session-1");

    let mut prompt = app.prompt.clone();
    prompt.set_input("/session".to_string());

    app.process_event(&Event::Custom(Box::new(
        CustomEvent::PromptSubmitRequested {
            prompt: Box::new(prompt),
        },
    )))
    .expect("session command should process through prompt submit");

    assert!(
        app.context.is_dialog_open(DialogSlot::SessionList),
        "session command should open the session list dialog even when already inside a session"
    );
}

#[test]
fn session_list_dialog_mouse_selection_switches_session() {
    let mut app = App::new().expect("app should initialize");
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: "session-1".to_string(),
            title: "First".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.upsert_session(Session {
            id: "session-2".to_string(),
            title: "Second".to_string(),
            created_at: now,
            updated_at: now - chrono::Duration::seconds(1),
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_messages(
            "session-1",
            vec![Message {
                id: "m1".to_string(),
                role: MessageRole::Assistant,
                content: "one".to_string(),
                created_at: now,
                agent: None,
                model: None,
                mode: None,
                finish: Some("stop".to_string()),
                error: None,
                completed_at: Some(now),
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: "one".to_string(),
                }],
            }],
        );
        session_ctx.set_messages(
            "session-2",
            vec![Message {
                id: "m2".to_string(),
                role: MessageRole::Assistant,
                content: "two".to_string(),
                created_at: now,
                agent: None,
                model: None,
                mode: None,
                finish: Some("stop".to_string()),
                error: None,
                completed_at: Some(now),
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: "two".to_string(),
                }],
            }],
        );
    }
    app.context.navigate_session("session-1");
    app.ensure_session_view("session-1");
    app.session_list_dialog.set_sessions(vec![
        SessionItem {
            id: "session-1".to_string(),
            title: "First".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            updated_at: now.timestamp_millis(),
            is_busy: false,
        },
        SessionItem {
            id: "session-2".to_string(),
            title: "Second".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            updated_at: (now - chrono::Duration::seconds(1)).timestamp_millis(),
            is_busy: false,
        },
    ]);
    app.open_session_list_dialog_modal(Some("session-1"));

    let area = Rect::new(0, 0, 120, 32);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    let mut surface = crate::ui::BufferSurface::new(&mut buffer);
    app.session_list_dialog
        .render_surface(&mut surface, area, &crate::theme::Theme::dark());
    let list_area = app
        .session_list_dialog
        .test_list_area()
        .expect("session list area should exist after render");
    let target_column = list_area.x;
    let target_row = list_area.y + 1;

    let consumed_down = app
        .handle_dialog_mouse(&crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: target_column,
            row: target_row,
            modifiers: KeyModifiers::NONE,
        })
        .expect("mouse down should process");
    let consumed_up = app
        .handle_dialog_mouse(&crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
            column: target_column,
            row: target_row,
            modifiers: KeyModifiers::NONE,
        })
        .expect("mouse up should process");

    assert!(consumed_down, "session list should consume mouse-down");
    assert!(consumed_up, "session list should consume mouse-up");
    assert_eq!(app.current_session_id().as_deref(), Some("session-2"));
    assert_eq!(
        app.context
            .session_view_handle()
            .as_ref()
            .map(|view| view.session_id()),
        Some("session-2")
    );
    assert!(
        !app.context.is_dialog_open(DialogSlot::SessionList),
        "session list should close after mouse selection switches session"
    );
}

#[test]
fn local_direct_idle_session_skips_session_fallback_deadlines() {
    let mut app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        ..AppLaunchConfig::default()
    })
    .expect("app should initialize");
    let session_id = "session-local-idle";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Local idle".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::Idle);
    }
    app.context.navigate_session(session_id);
    app.ensure_session_view(session_id);
    app.sync_runtime.last_question_sync = Instant::now() - Duration::from_secs(60);
    app.sync_runtime.last_permission_sync = Instant::now() - Duration::from_secs(60);
    app.sync_runtime.last_process_refresh = Instant::now() - Duration::from_secs(60);

    let deadline = app
        .next_tick_deadline(Instant::now())
        .expect("idle local session should still have non-session deadlines");

    assert!(deadline > Instant::now());
}

#[test]
fn local_direct_waiting_on_user_also_skips_session_fallback_deadlines() {
    let mut app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        ..AppLaunchConfig::default()
    })
    .expect("app should initialize");
    let session_id = "session-local-waiting";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Local waiting".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::WaitingOnUser);
    }
    app.context.navigate_session(session_id);
    app.ensure_session_view(session_id);
    app.sync_runtime.last_question_sync = Instant::now() - Duration::from_secs(60);
    app.sync_runtime.last_permission_sync = Instant::now() - Duration::from_secs(60);
    app.sync_runtime.last_process_refresh = Instant::now() - Duration::from_secs(60);

    let deadline = app
        .next_tick_deadline(Instant::now())
        .expect("waiting local session should still have non-session deadlines");

    assert!(deadline > Instant::now());
}

#[test]
fn local_direct_open_permission_prompt_does_not_rearm_permission_fallback() {
    let mut app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        ..AppLaunchConfig::default()
    })
    .expect("app should initialize");
    let session_id = "session-local-permission-open";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Local permission".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::WaitingOnUser);
    }
    app.context.navigate_session(session_id);
    app.ensure_session_view(session_id);
    app.permission_prompt.add_request(crate::components::PermissionRequest {
        id: "perm-1".to_string(),
        permission_type: crate::components::PermissionType::Bash,
        resource: "python3 demo.py".to_string(),
        tool_name: "bash".to_string(),
        permission_class: Some("dangerous_exec".to_string()),
        scope_key: None,
        scope_label: None,
        matcher_label: None,
        grant_target_summary: None,
        risk_tags: vec!["dangerous_exec".to_string()],
        supported_lifetimes: vec![crate::components::PermissionLifetime::Once],
        is_submitting: false,
        submit_error: None,
    });
    app.sync_runtime.last_permission_sync = Instant::now() - Duration::from_secs(60);

    let deadline = app
        .next_tick_deadline(Instant::now())
        .expect("permission prompt should not force an immediate fallback sync");

    assert!(deadline > Instant::now());
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
    assert!(!super::sync::session_update_requires_sync(Some(
        "direct_bridge"
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
    assert!(app.sync_runtime.pending_permission_sync_due_at.is_none());
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

    let event = Event::Custom(Box::new(CustomEvent::FrontendEvent(Box::new(
        FrontendEvent::PermissionUpsert {
            session_id: session_id.to_string(),
            permission: permission.clone(),
        },
    ))));

    app.handle_event(&event)
        .expect("permission requested event should be handled");

    assert!(app.event_caused_change);
    assert!(app.permission_runtime.pending_ids.contains("perm-1"));
    assert!(app.permission_prompt.is_open);
    assert!(app.sync_runtime.pending_permission_sync_due_at.is_none());
    assert_eq!(
        app.permission_runtime
            .pending_requests
            .get("perm-1")
            .map(|request| request.tool.as_str()),
        Some("bash")
    );
}

#[test]
fn waiting_on_user_runtime_snapshot_marks_session_as_waiting() {
    let mut app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        ..AppLaunchConfig::default()
    })
    .expect("app should initialize");
    let session_id = "session-waiting-state";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Waiting session".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::Running);
    }
    app.context.navigate_session(session_id);

    app.apply_frontend_event(&FrontendEvent::SessionRuntimeReplaced {
        session_id: session_id.to_string(),
        runtime: crate::api::SessionRuntimeState {
            session_id: session_id.to_string(),
            run_status: SessionRunStatusKind::WaitingOnUser,
            current_message_id: None,
            usage: None,
            active_stage_id: None,
            active_stage_count: 0,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            pending_followup_count: 0,
            attached_sessions: Vec::new(),
        },
    });

    assert!(matches!(
        app.active_session_status(),
        Some(SessionStatus::WaitingOnUser)
    ));
    assert!(app.local_direct_idle_session());
}

#[test]
fn question_upsert_event_populates_prompt_without_poll_sync() {
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

    let event = Event::Custom(Box::new(CustomEvent::FrontendEvent(Box::new(
        FrontendEvent::QuestionUpsert {
            session_id: session_id.to_string(),
            question: QuestionInfo {
                id: "q-1".to_string(),
                session_id: session_id.to_string(),
                questions: vec!["Proceed?".to_string()],
                options: Some(vec![vec!["Yes".to_string(), "No".to_string()]]),
                items: Vec::new(),
            },
        },
    ))));

    app.handle_event(&event)
        .expect("question upsert event should be handled");

    assert!(app.sync_runtime.pending_question_sync_due_at.is_none());
    assert_eq!(app.question_prompt.current().map(|q| q.id.as_str()), Some("q-1"));
}

#[test]
fn tool_call_upsert_start_updates_runtime_active_tools_without_telemetry_poll() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-tool-upsert-start";
    app.context.apply_session_runtime_snapshot(crate::api::SessionRuntimeState {
        session_id: session_id.to_string(),
        run_status: SessionRunStatusKind::Running,
        current_message_id: None,
        usage: None,
        active_stage_id: None,
        active_stage_count: 0,
        active_tools: Vec::new(),
        pending_question: None,
        pending_permission: None,
        pending_followup_count: 0,
        attached_sessions: Vec::new(),
    });
    {
        let mut session_ctx = app.context.session.write();
        let now = Utc::now();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Tool start".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);

    app.apply_frontend_event(&FrontendEvent::ToolCallUpsert {
        session_id: session_id.to_string(),
        tool_call_id: "tool-1".to_string(),
        tool_name: "bash".to_string(),
        phase: agendao_server_core::runtime_events::ToolCallPhase::Start,
    });

    let runtime = app.context.session_runtime().expect("runtime");
    assert_eq!(runtime.active_tools.len(), 1);
    assert_eq!(runtime.active_tools[0].tool_call_id, "tool-1");
    assert_eq!(runtime.active_tools[0].tool_name, "bash");
    assert!(app.sync_runtime.pending_session_telemetry_sync.is_none());
    assert!(!app.sync_runtime.session_telemetry_sync_inflight);
}

#[test]
fn tool_call_upsert_complete_removes_runtime_active_tool_without_telemetry_poll() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-tool-upsert-complete";
    app.context.apply_session_runtime_snapshot(crate::api::SessionRuntimeState {
        session_id: session_id.to_string(),
        run_status: SessionRunStatusKind::WaitingOnTool,
        current_message_id: None,
        usage: None,
        active_stage_id: None,
        active_stage_count: 0,
        active_tools: vec![crate::api::ActiveToolSummary {
            tool_call_id: "tool-1".to_string(),
            tool_name: "bash".to_string(),
            started_at: Utc::now().timestamp_millis(),
        }],
        pending_question: None,
        pending_permission: None,
        pending_followup_count: 0,
        attached_sessions: Vec::new(),
    });
    {
        let mut session_ctx = app.context.session.write();
        let now = Utc::now();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Tool complete".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);

    app.apply_frontend_event(&FrontendEvent::ToolCallUpsert {
        session_id: session_id.to_string(),
        tool_call_id: "tool-1".to_string(),
        tool_name: "bash".to_string(),
        phase: agendao_server_core::runtime_events::ToolCallPhase::Complete,
    });

    let runtime = app.context.session_runtime().expect("runtime");
    assert!(runtime.active_tools.is_empty());
    assert!(app.sync_runtime.pending_session_telemetry_sync.is_none());
    assert!(!app.sync_runtime.session_telemetry_sync_inflight);
}

#[test]
fn tool_call_upsert_start_creates_minimal_runtime_for_current_session() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-tool-upsert-placeholder";
    {
        let mut session_ctx = app.context.session.write();
        let now = Utc::now();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Tool placeholder".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);

    app.apply_frontend_event(&FrontendEvent::ToolCallUpsert {
        session_id: session_id.to_string(),
        tool_call_id: "tool-1".to_string(),
        tool_name: "bash".to_string(),
        phase: agendao_server_core::runtime_events::ToolCallPhase::Start,
    });

    let runtime = app.context.session_runtime().expect("runtime");
    assert_eq!(runtime.session_id, session_id);
    assert_eq!(runtime.run_status, SessionRunStatusKind::WaitingOnTool);
    assert_eq!(runtime.active_tools.len(), 1);
    assert_eq!(runtime.active_tools[0].tool_call_id, "tool-1");
    assert!(app.sync_runtime.pending_session_telemetry_sync.is_none());
    assert!(!app.sync_runtime.session_telemetry_sync_inflight);
}

#[test]
fn tool_call_upsert_non_current_session_updates_its_own_runtime_store() {
    let mut app = App::new().expect("app should initialize");
    let current_session_id = "session-current";
    let other_session_id = "session-other";
    {
        let mut session_ctx = app.context.session.write();
        let now = Utc::now();
        session_ctx.upsert_session(Session {
            id: current_session_id.to_string(),
            title: "Current".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.upsert_session(Session {
            id: other_session_id.to_string(),
            title: "Other".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(current_session_id.to_string());
    }
    app.context.navigate_session(current_session_id);

    app.apply_frontend_event(&FrontendEvent::ToolCallUpsert {
        session_id: other_session_id.to_string(),
        tool_call_id: "tool-1".to_string(),
        tool_name: "bash".to_string(),
        phase: agendao_server_core::runtime_events::ToolCallPhase::Start,
    });

    assert!(app.context.session_runtime().is_none());
    let other_runtime = app
        .context
        .session_runtime_for(other_session_id)
        .expect("other session runtime");
    assert_eq!(other_runtime.session_id, other_session_id);
    assert_eq!(other_runtime.run_status, SessionRunStatusKind::WaitingOnTool);
    assert_eq!(other_runtime.active_tools.len(), 1);
    assert_eq!(other_runtime.active_tools[0].tool_call_id, "tool-1");
    assert!(app.sync_runtime.pending_session_telemetry_sync.is_none());
    assert!(!app.sync_runtime.session_telemetry_sync_inflight);
}

#[test]
fn local_direct_permission_requested_event_does_not_queue_poll_sync() {
    let mut app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        ..AppLaunchConfig::default()
    })
    .expect("app should initialize");
    let session_id = "session-direct-permission";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Direct permission session".to_string(),
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
        id: "perm-direct".to_string(),
        session_id: session_id.to_string(),
        tool: "bash".to_string(),
        permission_class: Some("dangerous_exec".to_string()),
        scope_key: Some("cargo".to_string()),
        scope_label: Some("Shell commands: cargo".to_string()),
        origin_tool: None,
        supported_lifetimes: vec!["once".to_string()],
        matcher_kind: None,
        matcher_key: None,
        matcher_label: None,
        grant_target_summary: None,
        risk_tags: vec!["dangerous_exec".to_string()],
        input: serde_json::json!({
            "permission": "bash",
            "metadata": { "command": "cargo test" }
        }),
        message: "Execute cargo test".to_string(),
    };

    app.apply_frontend_event(&FrontendEvent::PermissionUpsert {
        session_id: session_id.to_string(),
        permission,
    });

    assert!(app.permission_prompt.is_open);
    assert!(app.sync_runtime.pending_permission_sync_due_at.is_none());
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

#[test]
fn pending_question_sync_deadline_suppresses_stale_fallback_deadline() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-question-debounce";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Question debounce".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);
    let now = Instant::now();
    let pending_due = now + Duration::from_millis(40);
    app.sync_runtime.last_question_sync = now - Duration::from_secs(60);
    app.sync_runtime.pending_question_sync_due_at = Some(pending_due);

    let deadline = app
        .next_tick_deadline(now)
        .expect("session route should produce a deadline");
    assert!(deadline >= now);
}

#[test]
fn pending_process_refresh_deadline_suppresses_stale_sidebar_fallback_deadline() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-process-debounce";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Process debounce".to_string(),
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
    app.toggle_session_sidebar();

    let now = Instant::now();
    let pending_due = now + Duration::from_millis(120);
    app.sync_runtime.last_process_refresh = now - Duration::from_secs(60);
    app.sync_runtime.pending_process_refresh_due_at = Some(pending_due);

    let deadline = app
        .next_tick_deadline(now)
        .expect("session route should produce a deadline");
    assert!(deadline <= pending_due);
}

#[test]
fn bridge_loop_snapshot_waits_when_idle_after_first_frame() {
    let app = App::new().expect("app should initialize");
    let now = Instant::now();

    let snapshot = app.bridge_loop_snapshot(now, false);

    assert!(matches!(
        snapshot.wait_strategy,
        BridgeWaitStrategy::Wait { .. }
    ));
}

#[test]
fn bridge_loop_snapshot_polls_ready_on_first_frame() {
    let app = App::new().expect("app should initialize");
    let now = Instant::now();

    let snapshot = app.bridge_loop_snapshot(now, true);

    assert_eq!(snapshot.wait_strategy, BridgeWaitStrategy::PollReady);
}

#[test]
fn bridge_iteration_skips_root_snapshot_when_no_draw_is_needed() {
    let mut app = App::new().expect("app should initialize");

    let outcome = app
        .process_bridge_iteration(None, false, 16, None, false)
        .expect("bridge iteration should succeed");

    assert!(!outcome.should_draw);
    assert!(outcome.reactive_root_snapshot.is_none());
}

#[test]
fn bridge_iteration_can_capture_root_snapshot_without_forcing_draw() {
    let mut app = App::new().expect("app should initialize");
    let area = Rect::new(0, 0, 100, 30);
    app.set_viewport_area(area);

    let outcome = app
        .process_bridge_iteration(None, false, 16, None, true)
        .expect("bridge iteration should succeed");

    assert!(!outcome.should_draw);
    let snapshot = outcome
        .reactive_root_snapshot
        .expect("snapshot should be captured when explicitly requested");
    assert_eq!(snapshot.route, Route::Home);
    assert_eq!(snapshot.prompt.get_input(), "");
}

#[test]
fn tick_does_not_rearm_question_sync_while_debounce_is_pending() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-question-rearm";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Question rearm".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
    }
    app.context.navigate_session(session_id);

    let pending_due = Instant::now() + Duration::from_millis(40);
    app.sync_runtime.last_question_sync = Instant::now() - Duration::from_secs(60);
    app.sync_runtime.pending_question_sync_due_at = Some(pending_due);

    app.handle_event(&Event::Tick)
        .expect("tick should preserve in-flight question debounce");

    assert_eq!(app.sync_runtime.pending_question_sync_due_at, Some(pending_due));
}

#[test]
fn local_direct_tick_does_not_rearm_session_question_or_permission_fallbacks() {
    let mut app = App::new_with_config(AppLaunchConfig {
        local_direct: true,
        ..AppLaunchConfig::default()
    })
    .expect("app should initialize");
    let session_id = "session-local-direct-no-fallback-rearm";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Local direct no fallback rearm".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::Running);
    }
    app.context.navigate_session(session_id);
    app.ensure_session_view(session_id);
    app.sync_runtime.last_question_sync = Instant::now() - Duration::from_secs(60);
    app.sync_runtime.last_permission_sync = Instant::now() - Duration::from_secs(60);
    app.sync_runtime.pending_session_sync = None;
    app.sync_runtime.pending_session_sync_due_at = None;
    app.sync_runtime.pending_session_telemetry_sync = None;
    app.sync_runtime.pending_session_telemetry_sync_due_at = None;

    app.handle_event(&Event::Tick)
        .expect("tick should not rearm direct-mode fallback syncs");

    assert!(app.sync_runtime.pending_session_sync.is_none());
    assert!(app.sync_runtime.pending_session_sync_due_at.is_none());
    assert!(app.sync_runtime.pending_question_sync_due_at.is_none());
    assert!(app.sync_runtime.pending_permission_sync_due_at.is_none());
}

#[test]
fn tick_does_not_rearm_process_refresh_while_debounce_is_pending() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-process-rearm";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Process rearm".to_string(),
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
    app.toggle_session_sidebar();

    let pending_due = Instant::now() + Duration::from_millis(120);
    app.sync_runtime.last_process_refresh = Instant::now() - Duration::from_secs(60);
    app.sync_runtime.pending_process_refresh_due_at = Some(pending_due);

    app.handle_event(&Event::Tick)
        .expect("tick should preserve in-flight process refresh debounce");

    assert_eq!(app.sync_runtime.pending_process_refresh_due_at, Some(pending_due));
}

#[test]
fn completed_session_tail_disables_spinner_even_if_local_status_is_running() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-completed-tail";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Completed tail".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_messages(
            session_id,
            vec![Message {
                id: "assistant-1".to_string(),
                role: MessageRole::Assistant,
                content: "done".to_string(),
                created_at: now,
                agent: None,
                model: None,
                mode: None,
                finish: Some("stop".to_string()),
                error: None,
                completed_at: Some(now),
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![crate::state::MessagePart::Text {
                    text: "done".to_string(),
                }],
            }],
        );
        session_ctx.set_status(session_id, SessionStatus::Running);
    }
    app.context.navigate_session(session_id);
    app.ensure_session_view(session_id);

    app.sync_prompt_spinner_state();
    assert!(!app.prompt.spinner_active());
}

#[test]
fn active_runtime_keeps_spinner_enabled_when_status_is_running() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-active-runtime";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Active runtime".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::Running);
    }
    app.context.navigate_session(session_id);
    app.ensure_session_view(session_id);
    app.context
        .apply_session_telemetry_snapshot(test_session_telemetry_snapshot(
            session_id,
            "stage-active",
        ));

    app.sync_prompt_spinner_state();
    assert!(app.prompt.spinner_active());
}

#[test]
fn waiting_on_user_pending_permission_keeps_spinner_disabled() {
    let mut app = App::new().expect("app should initialize");
    let session_id = "session-awaiting-permission";
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "Awaiting permission".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id(session_id.to_string());
        session_ctx.set_status(session_id, SessionStatus::WaitingOnUser);
    }
    app.context.navigate_session(session_id);
    app.ensure_session_view(session_id);
    let mut telemetry = test_session_telemetry_snapshot(session_id, "stage-awaiting");
    telemetry.runtime.run_status = SessionRunStatusKind::WaitingOnUser;
    telemetry.runtime.active_stage_id = None;
    telemetry.runtime.active_stage_count = 0;
    telemetry.runtime.pending_permission = Some(PendingPermissionSummary {
        permission_id: "perm-1".to_string(),
        requested_at: Utc::now().timestamp_millis(),
        tool: Some("bash".to_string()),
    });
    app.context.apply_session_telemetry_snapshot(telemetry);

    app.sync_prompt_spinner_state();
    assert!(!app.prompt.spinner_active());
}

#[test]
fn session_navigation_intent_parent_uses_existing_app_navigation_gate() {
    let mut app = App::new().expect("app should initialize");
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: "parent-session".to_string(),
            title: "Parent".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.upsert_session(Session {
            id: "child-session".to_string(),
            title: "Child".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: Some("parent-session".to_string()),
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id("child-session".to_string());
    }
    app.context.navigate_session("child-session");
    app.ensure_session_view("child-session");

    app.process_event(&Event::Custom(Box::new(CustomEvent::SessionNavigationIntent {
        kind: crate::event::SessionNavigationIntentKind::Parent,
    })))
    .expect("parent intent should process");

    assert_eq!(app.current_session_id().as_deref(), Some("parent-session"));
}

#[test]
fn session_navigation_intent_attached_uses_existing_app_navigation_gate() {
    let mut app = App::new().expect("app should initialize");
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: "root-session".to_string(),
            title: "Root".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.upsert_session(Session {
            id: "attached-session".to_string(),
            title: "Attached".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: Some("root-session".to_string()),
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id("root-session".to_string());
    }
    app.context.navigate_session("root-session");
    app.ensure_session_view("root-session");
    let mut telemetry = test_session_telemetry_snapshot("root-session", "stage-1");
    telemetry.stages = vec![StageSummary {
        stage_id: "stage-1".to_string(),
        stage_name: "Attached".to_string(),
        index: None,
        total: None,
        step: None,
        step_total: None,
        status: StageStatus::Running,
        prompt_tokens: None,
        context_tokens: None,
        completion_tokens: None,
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_miss_tokens: None,
        cache_write_tokens: None,
        focus: None,
        last_event: None,
        waiting_on: None,
        activity: None,
        estimated_context_tokens: None,
        skill_tree_budget: None,
        skill_tree_truncation_strategy: None,
        skill_tree_truncated: None,
        retry_attempt: None,
        active_agent_count: 0,
        active_tool_count: 0,
        attached_session_count: 1,
        primary_attached_session_id: Some("attached-session".to_string()),
    }];
    app.context.apply_session_telemetry_snapshot(telemetry);

    app.process_event(&Event::Custom(Box::new(CustomEvent::SessionNavigationIntent {
        kind: crate::event::SessionNavigationIntentKind::Attached,
    })))
    .expect("attached intent should process");

    assert_eq!(app.current_session_id().as_deref(), Some("attached-session"));
}

#[test]
fn session_navigation_intent_session_uses_existing_app_navigation_gate() {
    let mut app = App::new().expect("app should initialize");
    let now = Utc::now();
    {
        let mut session_ctx = app.context.session.write();
        session_ctx.upsert_session(Session {
            id: "session-1".to_string(),
            title: "One".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.upsert_session(Session {
            id: "session-2".to_string(),
            title: "Two".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_current_session_id("session-1".to_string());
        session_ctx.set_messages(
            "session-2",
            vec![Message {
                id: "m2".to_string(),
                role: MessageRole::Assistant,
                content: "two".to_string(),
                created_at: now,
                agent: None,
                model: None,
                mode: None,
                finish: Some("stop".to_string()),
                error: None,
                completed_at: Some(now),
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: "two".to_string(),
                }],
            }],
        );
    }
    app.context.navigate_session("session-1");
    app.ensure_session_view("session-1");

    app.process_event(&Event::Custom(Box::new(CustomEvent::SessionNavigationIntent {
        kind: crate::event::SessionNavigationIntentKind::Session("session-2".to_string()),
    })))
    .expect("session intent should process");

    assert_eq!(app.current_session_id().as_deref(), Some("session-2"));
    assert_eq!(
        app.context
            .session_view_handle()
            .as_ref()
            .map(|view| view.session_id()),
        Some("session-2")
    );
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
