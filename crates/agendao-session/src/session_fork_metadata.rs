use crate::session::{
    FORK_IMPORTED_HISTORY_METADATA_KEY, FORK_ORIGIN_MESSAGE_ID_METADATA_KEY,
    FORK_ORIGIN_SESSION_ID_METADATA_KEY, FORK_POLICY_FROZEN_METADATA_KEY,
};
use crate::{Session, SessionMessage};

const FORK_POLICY_METADATA_ALLOWLIST: &[&str] = &[
    "agent",
    "model_id",
    "model_provider",
    "model_variant",
    "scheduler_applied",
    "scheduler_profile",
    "scheduler_root_agent",
    "scheduler_selection_source",
    "scheduler_selection_trace",
    "scheduler_selection_warning",
    "scheduler_skill_tree_applied",
];

const FORK_CACHE_STABILITY_METADATA_KEYS: &[&str] = &[
    "agent",
    "model_id",
    "model_provider",
    "model_variant",
    "scheduler_applied",
    "scheduler_profile",
    "scheduler_root_agent",
    "scheduler_skill_tree_applied",
];

pub(crate) fn copy_fork_policy_metadata_from(source: &Session, target: &mut Session) {
    for key in FORK_POLICY_METADATA_ALLOWLIST {
        if let Some(value) = source.record().metadata.get(*key).cloned() {
            target.insert_metadata((*key).to_string(), value);
        }
    }
    target.insert_metadata(FORK_POLICY_FROZEN_METADATA_KEY, serde_json::json!(true));
}

pub(crate) fn fork_frozen_policy_keys(session: &Session) -> Vec<String> {
    FORK_POLICY_METADATA_ALLOWLIST
        .iter()
        .filter(|key| session.record().metadata.contains_key(**key))
        .map(|key| (*key).to_string())
        .collect()
}

pub(crate) fn fork_cache_stability_keys(session: &Session) -> Vec<String> {
    FORK_CACHE_STABILITY_METADATA_KEYS
        .iter()
        .filter(|key| session.record().metadata.contains_key(**key))
        .map(|key| (*key).to_string())
        .collect()
}

pub(crate) fn imported_fork_history_message(
    source_session_id: &str,
    target_session_id: &str,
    source_message: &SessionMessage,
) -> SessionMessage {
    let mut message = source_message.clone();
    message.session_id = target_session_id.to_string();
    message.metadata.insert(
        FORK_IMPORTED_HISTORY_METADATA_KEY.to_string(),
        serde_json::json!(true),
    );
    message
        .metadata
        .entry(FORK_ORIGIN_SESSION_ID_METADATA_KEY.to_string())
        .or_insert_with(|| serde_json::json!(source_session_id));
    message
        .metadata
        .entry(FORK_ORIGIN_MESSAGE_ID_METADATA_KEY.to_string())
        .or_insert_with(|| serde_json::json!(&source_message.id));

    let (admission, authority) = agendao_types::origin_to_admission_authority(
        agendao_types::MessageSourceOrigin::ImportedHistory,
    );
    agendao_types::apply_message_source_metadata(
        &mut message.metadata,
        agendao_types::MessageSourceOrigin::ImportedHistory,
        agendao_types::MessageSourceSurface::HttpApi,
    );
    agendao_types::apply_message_admission_metadata(&mut message.metadata, admission, authority);
    message
}
