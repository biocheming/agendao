use agendao_session::{MessageRole, Session, SessionContextKind, SessionManager, SessionForkSpec, SessionForkHistoryMode};
use agendao_types;

#[test]
fn test_session_creation() {
    let session = Session::new("test-project", "/test/directory");

    assert!(session.id.starts_with("ses_"));
    assert!(session.messages.is_empty());
    assert_eq!(session.project_id, "test-project");
    assert_eq!(session.directory, "/test/directory");
}

#[test]
fn test_session_add_user_message() {
    let mut session = Session::new("test-project", "/test/directory");

    session.add_user_message("Hello, world!");

    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].role, MessageRole::User);
}

#[test]
fn test_session_add_assistant_message() {
    let mut session = Session::new("test-project", "/test/directory");

    session.add_user_message("Hello");
    session.add_assistant_message();

    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.messages[0].role, MessageRole::User);
    assert_eq!(session.messages[1].role, MessageRole::Assistant);
}

#[test]
fn test_session_attached_creation() {
    let parent = Session::new("test-project", "/test/directory");
    let child =
        Session::attached_with_context_kind(&parent, SessionContextKind::DelegatedSubsession);

    assert!(child.parent_id.is_some());
    assert_eq!(child.parent_id.clone().unwrap(), parent.id);
    assert_eq!(child.project_id, parent.project_id);
    assert_eq!(child.directory, parent.directory);
    assert_eq!(
        parent.context_kind(),
        SessionContextKind::RootSessionContinuity
    );
    assert_eq!(
        child.context_kind(),
        SessionContextKind::DelegatedSubsession
    );
}

#[test]
fn test_session_default_title() {
    let session = Session::new("test-project", "/test/directory");

    assert!(session.is_default_title());

    let mut session_with_title = Session::new("test-project", "/test/directory");
    session_with_title.set_title("Custom Title");

    assert!(!session_with_title.is_default_title());
}

#[test]
fn test_session_forked_title() {
    let mut session = Session::new("test-project", "/test/directory");
    session.set_title("My Session");

    let forked = session.get_forked_title();
    assert_eq!(forked, "My Session (fork #1)");

    session.set_title("My Session (fork #1)");
    let forked2 = session.get_forked_title();
    assert_eq!(forked2, "My Session (fork #2)");
}

#[test]
fn test_fork_imported_history_messages_carry_imported_source_metadata() {
    let mut manager = SessionManager::new();
    let created = manager.create("test-project", "/test/dir");
    let sid = created.record().id.clone();

    // Add a user message to the source session so fork has something to import.
    manager.get_mut(&sid).unwrap().add_user_message("hello");

    // Fork with full history.
    let forked = manager
        .fork(
            &sid,
            SessionForkSpec {
                message_id: None,
                history_mode: SessionForkHistoryMode::All,
                history_message_limit: None,
            },
        )
        .expect("fork should succeed");

    // The forked session should contain an imported history copy of the source message.
    let imported_msg = forked
        .messages
        .iter()
        .find(|m| {
            m.metadata
                .get("fork_imported_history")
                .and_then(|v| v.as_bool())
                == Some(true)
        })
        .expect("forked session should have an imported history message");

    let origin = agendao_types::message_source_origin(&imported_msg.metadata);
    let surface = agendao_types::message_source_surface(&imported_msg.metadata);
    let admission = agendao_types::message_admission_context(&imported_msg.metadata);
    let authority = agendao_types::message_authority_class(&imported_msg.metadata);

    assert_eq!(origin, Some(agendao_types::MessageSourceOrigin::ImportedHistory));
    assert_eq!(surface, Some(agendao_types::MessageSourceSurface::HttpApi));
    assert_eq!(admission, Some(agendao_types::MessageAdmissionContext::Internal));
    assert_eq!(authority, Some(agendao_types::MessageAuthorityClass::System));
}

#[test]
fn test_system_message_carries_correct_admission_and_authority() {
    // Simulate steering/system message creation via the authority contract.
    let msg = agendao_session::SessionMessage::user_with_source(
        "test-session",
        "steering text",
        agendao_types::MessageSourceOrigin::System,
        agendao_types::MessageSourceSurface::HttpApi,
    );

    let origin = agendao_types::message_source_origin(&msg.metadata);
    let surface = agendao_types::message_source_surface(&msg.metadata);
    let admission = agendao_types::message_admission_context(&msg.metadata);
    let authority = agendao_types::message_authority_class(&msg.metadata);

    assert_eq!(origin, Some(agendao_types::MessageSourceOrigin::System));
    assert_eq!(surface, Some(agendao_types::MessageSourceSurface::HttpApi));
    assert_eq!(
        admission,
        Some(agendao_types::MessageAdmissionContext::Internal)
    );
    assert_eq!(
        authority,
        Some(agendao_types::MessageAuthorityClass::System)
    );
}

#[test]
fn test_session_touch_updates_timestamp() {
    let mut session = Session::new("test-project", "/test/directory");
    let original_time = session.time.updated;

    std::thread::sleep(std::time::Duration::from_millis(10));
    session.touch();

    assert!(session.time.updated >= original_time);
}

#[test]
fn test_session_message_id() {
    let mut session = Session::new("test-project", "/test/directory");

    session.add_user_message("Test message");
    assert!(!session.messages[0].id.is_empty());
    assert_eq!(session.messages[0].role, MessageRole::User);
}
