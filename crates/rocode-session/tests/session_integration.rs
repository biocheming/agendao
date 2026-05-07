use rocode_session::{MessageRole, Session, SessionContextKind};

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
fn test_session_child_creation() {
    let parent = Session::new("test-project", "/test/directory");
    let child = Session::child_with_context_kind(&parent, SessionContextKind::DelegatedSubsession);

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
