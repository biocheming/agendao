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

#[derive(Debug, Clone)]
pub(crate) struct RecentSessionRow {
    pub id: String,
    pub title: String,
    pub updated: i64,
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

pub(crate) async fn list_recent_session_rows(limit: i64) -> anyhow::Result<Vec<RecentSessionRow>> {
    let sessions = list_sessions(None, limit).await?;
    Ok(sessions
        .into_iter()
        .map(|session| RecentSessionRow {
            id: session.id,
            title: session.title,
            updated: session.time.updated,
        })
        .collect())
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
                let model = message
                    .metadata
                    .get("model_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                *model_usage
                    .entry(format!("{}/{}", provider, model))
                    .or_insert(0) += 1;
            }
            for part in message.parts {
                if let agendao_types::PartType::ToolCall { name, .. } = part.part_type {
                    *tool_usage.entry(name).or_insert(0) += 1;
                }
            }
        }
    }

    let mut last_run_status_usage_rows: Vec<_> = last_run_status_usage.into_iter().collect();
    last_run_status_usage_rows
        .sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut model_usage_rows: Vec<_> = model_usage.into_iter().collect();
    model_usage_rows.sort_by(|left, right| right.1.cmp(&left.1));
    if let Some(limit) = models_limit {
        model_usage_rows.truncate(limit);
    }

    let mut tool_usage_rows: Vec<_> = tool_usage.into_iter().collect();
    tool_usage_rows.sort_by(|left, right| right.1.cmp(&left.1));
    if let Some(limit) = tools_limit {
        tool_usage_rows.truncate(limit);
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
        last_run_status_usage: last_run_status_usage_rows,
        model_usage: model_usage_rows,
        tool_usage: tool_usage_rows,
    })
}

pub(crate) fn format_relative_session_time(timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let elapsed = now - timestamp;
    if elapsed < 0 {
        return "just now".to_string();
    }
    if elapsed < 60 {
        format!("{}s ago", elapsed)
    } else if elapsed < 3600 {
        format!("{}m ago", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{}h ago", elapsed / 3600)
    } else {
        format!("{}d ago", elapsed / 86400)
    }
}

fn session_stats_usage_summary(session: &Session) -> SessionStatsUsageSummary {
    if let Some(snapshot) = session
        .metadata
        .get(SESSION_TELEMETRY_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<SessionTelemetrySnapshot>(value).ok())
    {
        return SessionStatsUsageSummary {
            usage: snapshot.usage,
            used_persisted_snapshot: true,
            stage_summary_count: snapshot.stage_summaries.len(),
            last_run_status: Some(snapshot.last_run_status),
        };
    }

    SessionStatsUsageSummary {
        usage: session.usage.clone().unwrap_or_default(),
        used_persisted_snapshot: false,
        stage_summary_count: 0,
        last_run_status: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{session_stats_usage_summary, SESSION_TELEMETRY_METADATA_KEY};
    use std::collections::{BTreeMap, HashMap};

    use agendao_stage_protocol::StageStatus;
    use agendao_types::{
        PersistedStageTelemetrySummary, Session, SessionStatus, SessionTelemetrySnapshot,
        SessionTelemetrySnapshotVersion, SessionTime, SessionUsage,
    };

    fn sample_session() -> Session {
        Session {
            id: "session-1".to_string(),
            slug: "session-1".to_string(),
            project_id: "project".to_string(),
            directory: "/tmp/project".to_string(),
            parent_id: None,
            title: "Session".to_string(),
            version: "1".to_string(),
            time: SessionTime::default(),
            messages: Vec::new(),
            summary: None,
            share: None,
            revert: None,
            permission: None,
            usage: None,
            status: SessionStatus::Active,
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn stats_usage_prefers_persisted_snapshot_over_legacy_usage() {
        let mut session = sample_session();
        session.usage = Some(SessionUsage {
            input_tokens: 1,
            output_tokens: 2,
            reasoning_tokens: 3,
            cache_write_tokens: 4,
            cache_read_tokens: 5,
            cache_miss_tokens: 0,
            context_tokens: 0,
            total_cost: 0.1,
        });
        session.metadata.insert(
            SESSION_TELEMETRY_METADATA_KEY.to_string(),
            serde_json::to_value(SessionTelemetrySnapshot {
                version: SessionTelemetrySnapshotVersion::V1,
                usage: SessionUsage {
                    input_tokens: 100,
                    output_tokens: 200,
                    reasoning_tokens: 30,
                    cache_write_tokens: 40,
                    cache_read_tokens: 50,
                    cache_miss_tokens: 0,
                    context_tokens: 0,
                    total_cost: 1.5,
                },
                stage_summaries: vec![PersistedStageTelemetrySummary {
                    stage_id: "stage-1".to_string(),
                    stage_name: "Plan".to_string(),
                    index: Some(1),
                    total: Some(1),
                    step: Some(1),
                    step_total: Some(1),
                    status: StageStatus::Done,
                    prompt_tokens: None,
                    completion_tokens: None,
                    reasoning_tokens: None,
                    cache_read_tokens: None,
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
                    attached_session_count: 0,
                    primary_attached_session_id: None,
                }],
                tool_repair_summary: None,
                memory: None,
                compaction_continuity: None,
                repair_query_snapshot: None,
                tool_trajectory_quality: None,
                tool_result_governance: None,
                pending_permission_count: 0,
                pending_followup_count: 0,
                granted_by_turn_count: 0,
                granted_by_session_count: 0,
                granted_by_matcher_kind: BTreeMap::new(),
                last_permission_matcher_kind: None,
                last_permission_grant_target: None,
                last_permission_miss_count: 0,
                pending_steering_count: 0,
                consumed_steering_count: 0,
                last_steering_injected_at: None,
                last_steering_source_session_id: None,
                last_steering_latency_ms: None,
                last_permission_pending_ms: None,
                last_run_status: "completed".to_string(),
                updated_at: 123,
            })
            .expect("snapshot should serialize"),
        );

        let summary = session_stats_usage_summary(&session);

        assert!(summary.used_persisted_snapshot);
        assert_eq!(summary.usage.input_tokens, 100);
        assert_eq!(summary.stage_summary_count, 1);
        assert_eq!(summary.last_run_status.as_deref(), Some("completed"));
    }

    #[test]
    fn stats_usage_falls_back_to_legacy_usage_when_snapshot_missing() {
        let mut session = sample_session();
        session.usage = Some(SessionUsage {
            input_tokens: 10,
            output_tokens: 20,
            reasoning_tokens: 3,
            cache_write_tokens: 4,
            cache_read_tokens: 5,
            cache_miss_tokens: 0,
            context_tokens: 0,
            total_cost: 0.25,
        });

        let summary = session_stats_usage_summary(&session);

        assert!(!summary.used_persisted_snapshot);
        assert_eq!(summary.usage.output_tokens, 20);
        assert_eq!(summary.stage_summary_count, 0);
        assert_eq!(summary.last_run_status, None);
    }
}
