use std::process::Command as ProcessCommand;

use crate::cli::{DbCommands, DbOutputFormat};
use crate::cli_session_store::{collect_session_stats, local_database_path};

pub(super) async fn handle_db_command(
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

pub(super) async fn handle_stats_command(
    days: Option<i64>,
    tools_limit: Option<usize>,
    models_limit: Option<usize>,
    project: Option<String>,
) -> anyhow::Result<()> {
    let report = collect_session_stats(days, tools_limit, models_limit, project).await?;

    println!("Sessions: {}", report.sessions);
    println!("Messages: {}", report.messages);
    println!("Total Cost: ${:.4}", report.total_cost);
    println!(
        "Tokens: input={} output={} reasoning={} cache_read={} cache_miss={} cache_write={}",
        report.total_input,
        report.total_output,
        report.total_reasoning,
        report.total_cache_read,
        report.total_cache_miss,
        report.total_cache_write
    );
    println!(
        "Persisted telemetry: sessions={} stage_summaries={}",
        report.persisted_telemetry_sessions, report.persisted_stage_summaries
    );

    if !report.last_run_status_usage.is_empty() {
        println!("\nLast run status:");
        for (status, count) in report.last_run_status_usage {
            println!("  {:<20} {}", status, count);
        }
    }

    if !report.model_usage.is_empty() {
        println!("\nModel usage:");
        for (model, count) in report.model_usage {
            println!("  {:<40} {}", model, count);
        }
    }

    if !report.tool_usage.is_empty() {
        println!("\nTool usage:");
        for (tool, count) in report.tool_usage {
            println!("  {:<30} {}", tool, count);
        }
    }

    Ok(())
}
