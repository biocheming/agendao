use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{ApiError, Result, ServerState};

const BATCH_METADATA_KEY: &str = "live_web_ingress_batch";
pub(crate) const BATCH_WINDOW_MS: i64 = 250;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Batch {
    owner_turn_id: String,
    opened_at_ms: i64,
    items: Vec<BatchItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchItem {
    ingress: agendao_session::prompt::IngressTurnEnvelope,
    parts: Vec<agendao_session::prompt::PartInput>,
}

pub(crate) enum Stage {
    Bypass,
    Leader {
        owner_turn_id: String,
        reservation: CancellationToken,
    },
    Follower,
}

pub(super) fn supports(ingress: &agendao_session::prompt::IngressTurnEnvelope) -> bool {
    matches!(ingress.source, agendao_session::prompt::IngressSource::Web)
        && ingress.context_key.as_deref() == Some("session_prompt")
        && ingress.command.is_none()
}

fn load(session: &agendao_session::Session) -> Option<Batch> {
    session
        .metadata
        .get(BATCH_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn store(session: &mut agendao_session::Session, batch: &Batch) -> bool {
    match serde_json::to_value(batch) {
        Ok(value) => {
            session.insert_metadata(BATCH_METADATA_KEY.to_string(), value);
            true
        }
        Err(error) => {
            tracing::warn!(%error, "failed to serialize live web ingress batch");
            false
        }
    }
}

fn clear(session: &mut agendao_session::Session) {
    session.remove_metadata(BATCH_METADATA_KEY);
}

fn stale(batch: &Batch, now_ms: i64) -> bool {
    now_ms.saturating_sub(batch.opened_at_ms) > BATCH_WINDOW_MS
}

fn matches_batch(batch: &Batch, ingress: &agendao_session::prompt::IngressTurnEnvelope) -> bool {
    batch
        .items
        .first()
        .map(|first| {
            first.ingress.session_id == ingress.session_id
                && first.ingress.source == ingress.source
                && first.ingress.context_key == ingress.context_key
                && first.ingress.command == ingress.command
        })
        .unwrap_or(false)
}

pub(super) fn append_if_present(
    session: &mut agendao_session::Session,
    ingress: agendao_session::prompt::IngressTurnEnvelope,
    parts: Vec<agendao_session::prompt::PartInput>,
    now_ms: i64,
) -> bool {
    if !supports(&ingress) {
        return false;
    }

    let item = BatchItem { ingress, parts };
    let batch = load(session).filter(|batch| !stale(batch, now_ms));
    if batch.is_none() {
        clear(session);
    }

    if let Some(mut batch) = batch {
        if matches_batch(&batch, &item.ingress) {
            batch.items.push(item);
            return store(session, &batch);
        }
        clear(session);
    }

    false
}

pub(super) fn open(
    session: &mut agendao_session::Session,
    ingress: agendao_session::prompt::IngressTurnEnvelope,
    parts: Vec<agendao_session::prompt::PartInput>,
    now_ms: i64,
) -> Option<String> {
    if !supports(&ingress) {
        return None;
    }

    let item = BatchItem { ingress, parts };
    clear(session);
    let owner_turn_id = item.ingress.turn_id.clone();
    let batch = Batch {
        owner_turn_id: owner_turn_id.clone(),
        opened_at_ms: now_ms,
        items: vec![item],
    };
    store(session, &batch).then_some(owner_turn_id)
}

pub(super) fn drain(session: &mut agendao_session::Session, owner_turn_id: &str) -> Option<Batch> {
    let batch = load(session)?;
    if batch.owner_turn_id != owner_turn_id {
        return None;
    }
    clear(session);
    Some(batch)
}

pub(super) fn resolve(
    batch: Batch,
) -> Option<(
    agendao_session::prompt::IngressTurnEnvelope,
    Vec<agendao_session::prompt::PartInput>,
)> {
    let mut items = batch.items;
    items.sort_by(|left, right| {
        left.ingress
            .received_at_ms
            .cmp(&right.ingress.received_at_ms)
            .then_with(|| left.ingress.turn_id.cmp(&right.ingress.turn_id))
    });

    // Stabilization owns ingress-local merge semantics; authoritative prompt
    // content is rebuilt from PartInput below.
    let stabilized = agendao_session::prompt::stabilize_ingress_turns(
        items.iter().map(|item| item.ingress.clone()).collect(),
    );
    if stabilized.len() != 1 {
        tracing::warn!(
            item_count = items.len(),
            stabilized_count = stabilized.len(),
            "live web ingress batch did not stabilize to a single turn"
        );
        return None;
    }

    let mut seen_idempotency_keys = std::collections::HashSet::new();
    let mut merged_parts = Vec::new();
    for item in items {
        let duplicate = item
            .ingress
            .idempotency_key
            .as_deref()
            .map(|key| {
                let scoped = format!(
                    "{}:{:?}:{}",
                    item.ingress.session_id, item.ingress.source, key
                );
                !seen_idempotency_keys.insert(scoped)
            })
            .unwrap_or(false);
        if !duplicate {
            merged_parts.extend(item.parts);
        }
    }

    stabilized
        .into_iter()
        .next()
        .map(|ingress| (ingress, merged_parts))
}

pub(super) async fn stage(
    state: &Arc<ServerState>,
    session_id: &str,
    ingress: &agendao_session::prompt::IngressTurnEnvelope,
    parts: &[agendao_session::prompt::PartInput],
) -> Result<Stage> {
    if !supports(ingress) {
        return Ok(Stage::Bypass);
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    {
        let mut sessions = state.sessions.lock().await;
        let Some(mut session) = sessions.get(session_id).cloned() else {
            return Err(ApiError::SessionNotFound(session_id.to_string()));
        };
        if append_if_present(&mut session, ingress.clone(), parts.to_vec(), now_ms) {
            sessions.update(session);
            return Ok(Stage::Follower);
        }
        sessions.update(session);
    }

    let reservation = match state.prompt_runner.reserve_session_run(session_id).await {
        Ok(token) => token,
        Err(error) => {
            let mut sessions = state.sessions.lock().await;
            let Some(mut session) = sessions.get(session_id).cloned() else {
                return Err(ApiError::SessionNotFound(session_id.to_string()));
            };
            if append_if_present(&mut session, ingress.clone(), parts.to_vec(), now_ms) {
                sessions.update(session);
                return Ok(Stage::Follower);
            }
            return Err(ApiError::BadRequest(error.to_string()));
        }
    };

    let mut sessions = state.sessions.lock().await;
    let Some(mut session) = sessions.get(session_id).cloned() else {
        drop(sessions);
        state
            .prompt_runner
            .release_reserved_session_run(session_id)
            .await;
        return Err(ApiError::SessionNotFound(session_id.to_string()));
    };

    if append_if_present(&mut session, ingress.clone(), parts.to_vec(), now_ms) {
        sessions.update(session);
        drop(sessions);
        state
            .prompt_runner
            .release_reserved_session_run(session_id)
            .await;
        return Ok(Stage::Follower);
    }

    let Some(owner_turn_id) = open(&mut session, ingress.clone(), parts.to_vec(), now_ms) else {
        sessions.update(session);
        drop(sessions);
        state
            .prompt_runner
            .release_reserved_session_run(session_id)
            .await;
        return Ok(Stage::Bypass);
    };

    sessions.update(session);
    Ok(Stage::Leader {
        owner_turn_id,
        reservation,
    })
}
