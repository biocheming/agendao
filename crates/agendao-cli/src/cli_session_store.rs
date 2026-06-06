use std::collections::BTreeMap;
use std::path::PathBuf;

use agendao_session::SESSION_TELEMETRY_METADATA_KEY;
use agendao_storage::{Database, MessageRepository, SessionRepository};
use agendao_types::{
    Session, SessionArtifactBundle, SessionArtifactEntry, SessionArtifactImportEnvelope,
    SessionTelemetrySnapshot, SessionUsage,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionStatsReport {
    pub sessions: usize,
    pub messages: usize,
    pub total_cost: f64,
    pub total_input: u64,
    pub total_output: u64,
    pub total_reasoning: u64,
    pub total_cache_read: u64,
    pub total_cache_miss: u64,
    pub total_cache_write: u64,
    pub persisted_telemetry_sessions: usize,
    pub persisted_stage_summaries: usize,
    pub last_run_status_usage: Vec<(String, usize)>,
    pub model_usage: Vec<(String, usize)>,
    pub tool_usage: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionDetailRow {
    pub session: Session,
    pub message_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct SessionStatsUsageSummary {
    usage: SessionUsage,
    used_persisted_snapshot: bool,
    stage_summary_count: usize,
    last_run_status: Option<String>,
}

pub(crate) fn local_database_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agendao")
        .join("agendao.db")
}

pub(crate) async fn list_sessions(
    project: Option<&str>,
    limit: i64,
) -> anyhow::Result<Vec<Session>> {
    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    Ok(session_repo.list(project, limit).await?)
}

pub(crate) async fn get_session_detail(
    session_id: &str,
) -> anyhow::Result<Option<SessionDetailRow>> {
    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());

    let Some(session) = session_repo.get(session_id).await? else {
        return Ok(None);
    };
    let messages = message_repo.list_for_session(session_id).await?;
    Ok(Some(SessionDetailRow {
        session,
        message_count: messages.len(),
    }))
}

pub(crate) async fn delete_session(session_id: &str) -> anyhow::Result<()> {
    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());
    message_repo.delete_for_session(session_id).await?;
    session_repo.delete(session_id).await?;
    Ok(())
}

pub(crate) async fn export_session_bundle(
    session_id: Option<&str>,
) -> anyhow::Result<SessionArtifactBundle> {
    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());

    let session = if let Some(session_id) = session_id {
        session_repo
            .get(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?
    } else {
        session_repo
            .list(None, 1)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No sessions found to export"))?
    };

    let messages = message_repo.list_for_session(&session.id).await?;
    Ok(SessionArtifactBundle::new_now(vec![
        SessionArtifactEntry::new(session, messages),
    ]))
}

pub(crate) async fn import_session_bundle(
    payload: SessionArtifactImportEnvelope,
) -> anyhow::Result<usize> {
    let entries = payload.into_entries();
    if entries.is_empty() {
        anyhow::bail!("No session entries found in import payload");
    }

    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());

    let mut imported = 0usize;
    for mut entry in entries {
        let session_id = entry.session.id.clone();
        entry.session.messages.clear();

        if session_repo.get(&entry.session.id).await?.is_some() {
            session_repo.update(&entry.session).await?;
        } else {
            session_repo.create(&entry.session).await?;
        }

        for mut message in entry.messages {
            if message.session_id.is_empty() {
                message.session_id = session_id.clone();
            }
            message_repo.upsert(&message).await?;
        }
        imported += 1;
    }

    Ok(imported)
}

pub(crate) async fn collect_session_stats(
    days: Option<i64>,
    tools_limit: Option<usize>,
    models_limit: Option<usize>,
    project: Option<String>,
) -> anyhow::Result<SessionStatsReport> {
    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());

    let mut sessions = session_repo.list(None, 50_000).await?;
    if let Some(project) = project {
        if project.is_empty() {
            let cwd = std::env::current_dir()?.display().to_string();
            sessions.retain(|session| session.directory == cwd);
        } else {
            sessions.retain(|session| session.project_id == project);
        }
    }

    if let Some(days) = days {
        let now = chrono::Utc::now().timestamp_millis();
        let cutoff = if days == 0 {
            let dt = chrono::Utc::now()
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
                .timestamp_millis()
        } else {
            now - (days * 24 * 60 * 60 * 1000)
        };
        sessions.retain(|session| session.time.updated >= cutoff);
    }

    let mut total_messages = 0usize;
    let mut total_cost = 0.0f64;
    let mut total_input = 0u64;
    let mut total_output = 0u64;
    let mut total_reasoning = 0u64;
    let mut total_cache_read = 0u64;
    let mut total_cache_miss = 0u64;
    let mut total_cache_write = 0u64;
    let mut persisted_telemetry_sessions = 0usize;
    let mut persisted_stage_summaries = 0usize;
    let mut last_run_status_usage: BTreeMap<String, usize> = BTreeMap::new();
    let mut tool_usage: BTreeMap<String, usize> = BTreeMap::new();
    let mut model_usage: BTreeMap<String, usize> = BTreeMap::new();

    for session in &sessions {
        let usage_summary = session_stats_usage_summary(session);
        total_cost += usage_summary.usage.total_cost;
        total_input += usage_summary.usage.input_tokens;
        total_output += usage_summary.usage.output_tokens;
        total_reasoning += usage_summary.usage.reasoning_tokens;
        total_cache_read += usage_summary.usage.cache_read_tokens;
        total_cache_miss += usage_summary.usage.cache_miss_tokens;
        total_cache_write += usage_summary.usage.cache_write_tokens;
        if usage_summary.used_persisted_snapshot {
            persisted_telemetry_sessions += 1;
            persisted_stage_summaries += usage_summary.stage_summary_count;
            if let Some(status) = usage_summary.last_run_status {
                *last_run_status_usage.entry(status).or_insert(0) += 1;
            }
        }

        let messages = message_repo.list_for_session(&session.id).await?;
        total_messages += messages.len();

        for message in messages {
            if let Some(provider) = message
                .metadata
                .get("provider_id")
                .and_then(|value| value.as_str())
            {
                let model_name = message
                    .metadata
                    .get("model")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let key = if model_name.is_empty() {
                    provider.to_string()
                } else {
                    format!("{provider}/{model_name}")
                };
                *model_usage.entry(key).or_insert(0) += 1;
            }

            if let Some(tool_calls) = message
                .metadata
                .get("tool_calls")
                .and_then(|value| value.as_array())
            {
                for tool_call in tool_calls {
                    if let Some(name) = tool_call.get("name").and_then(|value| value.as_str()) {
                        *tool_usage.entry(name.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let mut model_usage: Vec<_> = model_usage.into_iter().collect();
    model_usage.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if let Some(limit) = models_limit {
        model_usage.truncate(limit);
    }

    let mut tool_usage: Vec<_> = tool_usage.into_iter().collect();
    tool_usage.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if let Some(limit) = tools_limit {
        tool_usage.truncate(limit);
    }

    Ok(SessionStatsReport {
        sessions: sessions.len(),
        messages: total_messages,
        total_cost,
        total_input,
        total_output,
        total_reasoning,
        total_cache_read,
        total_cache_miss,
        total_cache_write,
        persisted_telemetry_sessions,
        persisted_stage_summaries,
        last_run_status_usage: last_run_status_usage.into_iter().collect(),
        model_usage,
        tool_usage,
    })
}

fn session_stats_usage_summary(session: &Session) -> SessionStatsUsageSummary {
    let persisted_snapshot = session
        .metadata
        .get(SESSION_TELEMETRY_METADATA_KEY)
        .and_then(|value| serde_json::from_value::<SessionTelemetrySnapshot>(value.clone()).ok());

    let stage_summary_count = persisted_snapshot
        .as_ref()
        .map(|snapshot| snapshot.stage_summaries.len())
        .unwrap_or(0);
    let last_run_status = persisted_snapshot
        .as_ref()
        .map(|snapshot| snapshot.last_run_status.clone());

    let usage = persisted_snapshot
        .as_ref()
        .map(|snapshot| snapshot.usage.clone())
        .or_else(|| session.usage.clone())
        .unwrap_or_default();

    SessionStatsUsageSummary {
        usage,
        used_persisted_snapshot: persisted_snapshot.is_some(),
        stage_summary_count,
        last_run_status,
    }
}
