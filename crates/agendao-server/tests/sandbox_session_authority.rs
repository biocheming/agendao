//! Session-scoped sandbox-authority regression tests.
//!
//! These exercise the public session authority path rather than constructing
//! policy inputs by hand: each launch receives an immutable mode snapshot and
//! later permission edits affect only later launches.

mod support;

use std::sync::Arc;

use agendao_server::{local_create_session, local_set_session_permission_mode, ServerState};
use agendao_types::SessionPermissionMode;

async fn create_session(state: Arc<ServerState>, workspace: &std::path::Path) -> String {
    local_create_session(
        state,
        agendao_api::CreateSessionRequest {
            scheduler: None,
            directory: Some(workspace.to_string_lossy().into_owned()),
            project_id: Some("sandbox-session-test".to_string()),
            title: None,
        },
    )
    .await
    .expect("create session")
    .id
}

#[tokio::test]
async fn session_authorities_are_isolated_and_permission_edits_rebind_future_launches() {
    let workspace = support::test_root("sandbox_session_authority");
    let state = Arc::new(ServerState::new_for_workspace(workspace.clone()));
    let first = create_session(state.clone(), &workspace).await;
    let second = create_session(state.clone(), &workspace).await;

    let first_default = state.sandbox_authority_for_session(&first).await;
    assert_eq!(first_default.session_mode(), SessionPermissionMode::Default);

    local_set_session_permission_mode(
        state.clone(),
        &first,
        SessionPermissionMode::TrustedWorkspace,
    )
    .await
    .expect("set trusted mode");
    local_set_session_permission_mode(
        state.clone(),
        &second,
        SessionPermissionMode::UnsandboxedYolo,
    )
    .await
    .expect("set yolo mode");

    let first_trusted = state.sandbox_authority_for_session(&first).await;
    let second_yolo = state.sandbox_authority_for_session(&second).await;
    assert_eq!(
        first_trusted.session_mode(),
        SessionPermissionMode::TrustedWorkspace
    );
    assert_eq!(
        second_yolo.session_mode(),
        SessionPermissionMode::UnsandboxedYolo
    );
    assert_eq!(
        first_default.session_mode(),
        SessionPermissionMode::Default,
        "an authority already handed to an execution remains immutable"
    );

    local_set_session_permission_mode(
        state.clone(),
        &first,
        SessionPermissionMode::UnsandboxedYolo,
    )
    .await
    .expect("change first session mode");
    let first_yolo = state.sandbox_authority_for_session(&first).await;
    assert_eq!(
        first_yolo.session_mode(),
        SessionPermissionMode::UnsandboxedYolo
    );
    assert_eq!(
        first_trusted.session_mode(),
        SessionPermissionMode::TrustedWorkspace,
        "permission change only affects later launches"
    );
}

#[test]
fn pty_and_bash_share_the_single_native_mode_mapping() {
    assert!(!ServerState::sandbox_native_allowed_for_mode(
        SessionPermissionMode::Default
    ));
    assert!(!ServerState::sandbox_native_allowed_for_mode(
        SessionPermissionMode::TrustedWorkspace
    ));
    assert!(ServerState::sandbox_native_allowed_for_mode(
        SessionPermissionMode::UnsandboxedYolo
    ));
}
