use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use rocode_session::SESSION_TELEMETRY_METADATA_KEY;
use rocode_storage::{Database, MessageRepository, SessionRepository};

use crate::cli::{DbCommands, DbOutputFormat};

fn local_database_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rocode")
        .join("rocode.db")
}

pub(crate) async fn handle_db_command(
    action: Option<DbCommands>,
    query: Option<String>,
    format: DbOutputFormat,
) -> anyhow::Result<()> {
    if matches!(action, Some(DbCommands::Path)) {
        println!("{}", local_database_path().display());
        return Ok(());
    }

    let db_path = local_database_path();
    if let Some(query) = query {
        let mut args = vec![db_path.display().to_string()];
        match format {
            DbOutputFormat::Json => args.push("-json".to_string()),
            DbOutputFormat::Tsv => args.push("-tabs".to_string()),
        }
        args.push(query);

        let output = ProcessCommand::new("sqlite3")
            .args(&args)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run sqlite3: {}", e))?;
        if output.status.success() {
            print!("{}", String::from_utf8_lossy(&output.stdout));
            return Ok(());
        }
        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr));
    }

    let status = ProcessCommand::new("sqlite3")
        .arg(db_path)
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to run sqlite3 interactive shell: {}", e))?;
    if !status.success() {
        anyhow::bail!("sqlite3 exited with status {}", status);
    }
    Ok(())
}

pub(crate) async fn handle_stats_command(
    days: Option<i64>,
    tools_limit: Option<usize>,
    models_limit: Option<usize>,
    project: Option<String>,
) -> anyhow::Result<()> {
    let db = Database::new().await?;
    let session_repo = SessionRepository::new(db.pool().clone());
    let message_repo = MessageRepository::new(db.pool().clone());

    let mut sessions = session_repo.list(None, 50_000).await?;
    if let Some(project) = project {
        if project.is_empty() {
            let cwd = std::env::current_dir()?.display().to_string();
            sessions.retain(|s| s.directory == cwd);
        } else {
            sessions.retain(|s| s.project_id == project);
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
        sessions.retain(|s| s.time.updated >= cutoff);
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
            if let Some(provider) = message.metadata.get("provider_id").and_then(|v| v.as_str()) {
                let model = message
                    .metadata
                    .get("model_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                *model_usage
                    .entry(format!("{}/{}", provider, model))
                    .or_insert(0) += 1;
            }
            for part in message.parts {
                if let rocode_types::PartType::ToolCall { name, .. } = part.part_type {
                    *tool_usage.entry(name).or_insert(0) += 1;
                }
            }
        }
    }

    println!("Sessions: {}", sessions.len());
    println!("Messages: {}", total_messages);
    println!("Total Cost: ${:.4}", total_cost);
    println!(
        "Tokens: input={} output={} reasoning={} cache_read={} cache_miss={} cache_write={}",
        total_input,
        total_output,
        total_reasoning,
        total_cache_read,
        total_cache_miss,
        total_cache_write
    );
    println!(
        "Persisted telemetry: sessions={} stage_summaries={}",
        persisted_telemetry_sessions, persisted_stage_summaries
    );

    if !last_run_status_usage.is_empty() {
        println!("\nLast run status:");
        let mut rows: Vec<_> = last_run_status_usage.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (status, count) in rows {
            println!("  {:<20} {}", status, count);
        }
    }

    if !model_usage.is_empty() {
        println!("\nModel usage:");
        let mut rows: Vec<_> = model_usage.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        if let Some(limit) = models_limit {
            rows.truncate(limit);
        }
        for (model, count) in rows {
            println!("  {:<40} {}", model, count);
        }
    }

    if !tool_usage.is_empty() {
        println!("\nTool usage:");
        let mut rows: Vec<_> = tool_usage.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        if let Some(limit) = tools_limit {
            rows.truncate(limit);
        }
        for (tool, count) in rows {
            println!("  {:<30} {}", tool, count);
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
struct SessionStatsUsageSummary {
    usage: rocode_types::SessionUsage,
    used_persisted_snapshot: bool,
    stage_summary_count: usize,
    last_run_status: Option<String>,
}

fn session_stats_usage_summary(session: &rocode_types::Session) -> SessionStatsUsageSummary {
    if let Some(snapshot) = session
        .metadata
        .get(SESSION_TELEMETRY_METADATA_KEY)
        .cloned()
        .and_then(|value| {
            serde_json::from_value::<rocode_types::SessionTelemetrySnapshot>(value).ok()
        })
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
    use super::session_stats_usage_summary;
    use std::collections::HashMap;

    use rocode_command::stage_protocol::StageStatus;
    use rocode_types::{
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
            rocode_session::SESSION_TELEMETRY_METADATA_KEY.to_string(),
            serde_json::to_value(SessionTelemetrySnapshot {
                version: SessionTelemetrySnapshotVersion::V1,
                usage: rocode_types::SessionUsage {
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
                    estimated_context_tokens: None,
                    skill_tree_budget: None,
                    skill_tree_truncation_strategy: None,
                    skill_tree_truncated: None,
                    retry_attempt: None,
                    active_agent_count: 0,
                    active_tool_count: 0,
                    child_session_count: 0,
                    primary_child_session_id: None,
                }],
                memory: None,
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
