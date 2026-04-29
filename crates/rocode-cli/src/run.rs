use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rocode_agent::{AgentInfo, AgentRegistry};
#[cfg(test)]
use rocode_command::cli_panel::{
    display_width, pad_right_display, truncate_display, wrap_display_text,
};
use rocode_command::cli_permission::{prompt_permission, PermissionDecision, PermissionMemory};
use rocode_command::cli_prompt::{
    PromptCompletion, PromptFrame, PromptSession, PromptSessionEvent,
};
use rocode_command::cli_select::{
    interactive_multi_select, interactive_select, SelectOption, SelectResult,
};
use rocode_command::cli_spinner::SpinnerGuard;
use rocode_command::cli_style::CliStyle;
use rocode_command::interactive::{parse_interactive_command, InteractiveCommand};
use rocode_command::output_blocks::{
    render_cli_block_rich, MessageBlock, MessagePhase, MessageRole as OutputMessageRole,
    OutputBlock, QueueItemBlock, SchedulerStageBlock, StatusBlock,
};
use rocode_command::terminal_presentation::{
    render_terminal_stream_block_semantic, TerminalSemanticStreamRenderState,
    TerminalStreamAccumulator,
};
use rocode_command::{CommandRegistry, ResolvedUiCommand, UiActionId};
use rocode_config::loader::load_config;
use rocode_config::Config;
use rocode_core::agent_task_registry::{global_task_registry, AgentTaskStatus};
use rocode_orchestrator::{
    scheduler_auto_profile_config, scheduler_plan_from_profile,
    scheduler_request_defaults_from_plan, SchedulerConfig, SchedulerPresetKind,
    SchedulerProfileConfig, SchedulerRequestDefaults, AUTO_SCHEDULER_PROFILE_NAME,
};
use rocode_provider::ProviderRegistry;
use rocode_util::util::color::strip_ansi;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

use crate::api_client::{
    CliApiClient, McpStatusInfo, SessionExecutionTopology, SessionRuntimeState,
};
use crate::branding::{APP_SHORT_NAME, APP_TAGLINE, APP_VERSION_DATE};
use crate::cli::{InteractiveCliMode, RunOutputFormat};
use crate::clipboard::Clipboard;
use crate::event_stream::{self, CliServerEvent};
use crate::providers::{render_help, setup_providers_for_dir};
use crate::remote::{parse_output_block, run_non_interactive_attach, RemoteAttachOptions};
use crate::server_lifecycle::FrontendRuntimeContext;
use crate::util::{
    append_cli_file_attachments, collect_run_input, parse_model_and_provider, truncate_text,
};
use rocode_command::branding::logo_lines;

mod interactive_session;

fn resolve_requested_agent_name(
    config: &Config,
    requested_agent: Option<&str>,
    scheduler_defaults: Option<&SchedulerRequestDefaults>,
) -> String {
    if let Some(agent) = requested_agent
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return agent.to_string();
    }

    if let Some(agent) = scheduler_defaults.and_then(|defaults| defaults.root_agent_name.clone()) {
        return agent;
    }

    if let Some(agent) = config
        .default_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return agent.to_string();
    }

    "build".to_string()
}

fn cli_resolve_show_thinking(explicit_flag: bool, config: Option<&Config>, fallback: bool) -> bool {
    if explicit_flag {
        return true;
    }

    config
        .and_then(|cfg| cfg.ui_preferences.as_ref())
        .and_then(|ui| ui.show_thinking)
        .unwrap_or(fallback)
}

async fn cli_save_recent_model_ref(api_client: &CliApiClient, model_ref: &str) {
    let Some((provider, model)) = model_ref.split_once('/') else {
        return;
    };
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() || model.is_empty() {
        return;
    }
    let mut recent = api_client.get_recent_models().await.unwrap_or_default();
    recent.retain(|entry| {
        !(entry.provider.eq_ignore_ascii_case(provider) && entry.model.eq_ignore_ascii_case(model))
    });
    recent.insert(
        0,
        rocode_state::RecentModelEntry {
            provider: provider.to_string(),
            model: model.to_string(),
        },
    );
    recent.truncate(rocode_state::MAX_RECENT_MODELS);
    if let Err(error) = api_client.put_recent_models(&recent).await {
        tracing::warn!(%error, "failed to persist CLI recent model");
    }
}

pub(crate) async fn run_non_interactive(
    options: RunNonInteractiveOptions,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    let RunNonInteractiveOptions {
        message,
        command,
        continue_last,
        session,
        fork,
        share,
        model,
        requested_agent,
        requested_scheduler_profile,
        files,
        format,
        title,
        attach,
        dir,
        port,
        variant,
        thinking,
        interactive_mode,
    } = options;
    let working_dir = match dir {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);

    if fork && !continue_last && session.is_none() {
        anyhow::bail!("--fork requires --continue or --session");
    }

    let mut input = collect_run_input(message)?;
    append_cli_file_attachments(&mut input, &files, &working_dir)?;
    if input.trim().is_empty() {
        let (provider, model_id) = parse_model_and_provider(model);
        return interactive_session::run_chat_session(
            model_id,
            provider,
            requested_agent,
            requested_scheduler_profile,
            thinking,
            interactive_mode,
            working_dir,
            runtime_context,
        )
        .await;
    }

    let base_url = if let Some(base_url) = attach {
        base_url
    } else {
        runtime_context
            .discover_or_start_server_with_request(crate::ServerDiscoveryRequest {
                port_override: port,
                cwd: Some(working_dir.clone()),
            })
            .await?
    };
    let api_client = CliApiClient::new(base_url.clone());
    let remote_context = api_client.get_workspace_context().await.ok();
    let show_thinking = cli_resolve_show_thinking(
        thinking,
        remote_context.as_ref().map(|context| &context.config),
        false,
    );
    let model = model.or_else(|| {
        remote_context
            .as_ref()
            .and_then(|context| context.recent_models.first())
            .map(|entry| format!("{}/{}", entry.provider, entry.model))
    });
    if let Some(model_ref) = model.as_deref() {
        cli_save_recent_model_ref(&api_client, model_ref).await;
    }

    run_non_interactive_attach(RemoteAttachOptions {
        base_url,
        input,
        command,
        continue_last,
        session,
        fork,
        share,
        model,
        agent: requested_agent,
        scheduler_profile: requested_scheduler_profile,
        variant,
        format,
        title,
        directory: Some(cli_session_directory(&working_dir)),
        show_thinking,
    })
    .await
}

pub(crate) struct RunNonInteractiveOptions {
    pub message: Vec<String>,
    pub command: Option<String>,
    pub continue_last: bool,
    pub session: Option<String>,
    pub fork: bool,
    pub share: bool,
    pub model: Option<String>,
    pub requested_agent: Option<String>,
    pub requested_scheduler_profile: Option<String>,
    pub files: Vec<PathBuf>,
    pub format: RunOutputFormat,
    pub title: Option<String>,
    pub attach: Option<String>,
    pub dir: Option<PathBuf>,
    pub port: Option<u16>,
    pub variant: Option<String>,
    pub thinking: bool,
    pub interactive_mode: InteractiveCliMode,
}

#[derive(Debug, Clone, Default)]
struct CliRunSelection {
    model: Option<String>,
    provider: Option<String>,
    requested_agent: Option<String>,
    requested_scheduler_profile: Option<String>,
    show_thinking: bool,
}

struct CliExecutionRuntime {
    resolved_agent_name: String,
    resolved_scheduler_profile_name: Option<String>,
    resolved_model_label: String,
    working_dir: PathBuf,
    observed_topology: Arc<Mutex<CliObservedExecutionTopology>>,
    frontend_projection: Arc<Mutex<CliFrontendProjection>>,
    scheduler_stage_snapshots: Arc<Mutex<HashMap<String, String>>>,
    terminal_surface: Option<Arc<CliTerminalSurface>>,
    prompt_chrome: Option<Arc<CliPromptChrome>>,
    prompt_session: Option<Arc<PromptSession>>,
    prompt_session_slot: Arc<std::sync::Mutex<Option<Arc<PromptSession>>>>,
    queued_inputs: Arc<AsyncMutex<VecDeque<String>>>,
    busy_flag: Arc<AtomicBool>,
    exit_requested: Arc<AtomicBool>,
    active_abort: Arc<AsyncMutex<Option<CliActiveAbortHandle>>>,
    recovery_base_prompt: Option<String>,
    /// Shared spinner guard — updated each message cycle so that question/permission
    /// callbacks can pause the active spinner without holding a stale reference.
    spinner_guard: Arc<std::sync::Mutex<SpinnerGuard>>,
    /// HTTP client for communicating with the server (Phase 3 unification).
    api_client: Option<Arc<CliApiClient>>,
    /// Server-side session ID (created via HTTP POST /session).
    server_session_id: Option<String>,
    /// Root session plus any explicitly attached child sessions for the active execution tree.
    related_session_ids: Arc<Mutex<BTreeSet<String>>>,
    /// Canonical retained transcript for the root session even when the operator
    /// temporarily focuses a child session view.
    root_session_transcript: Arc<Mutex<CliRetainedTranscript>>,
    /// Background transcripts for non-root child sessions. These are populated
    /// from the unified event surface but not rendered into the main transcript
    /// until the operator explicitly focuses one.
    child_session_transcripts: Arc<Mutex<HashMap<String, CliRetainedTranscript>>>,
    stream_accumulators: Arc<Mutex<HashMap<String, TerminalStreamAccumulator>>>,
    render_states: Arc<Mutex<HashMap<String, TerminalSemanticStreamRenderState>>>,
    /// Local CLI-only focus target. `None` means the root session remains visible.
    focused_session_id: Arc<Mutex<Option<String>>>,
    permission_memory: Arc<AsyncMutex<PermissionMemory>>,
    show_thinking: Arc<AtomicBool>,
}

fn cli_session_directory(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

struct CliRuntimeBuildInput<'a> {
    config: &'a Config,
    agent_registry: Arc<AgentRegistry>,
    selection: &'a CliRunSelection,
    working_dir: PathBuf,
}

#[derive(Clone)]
struct CliInteractiveHandles {
    terminal_surface: Arc<CliTerminalSurface>,
    prompt_chrome: Arc<CliPromptChrome>,
    prompt_session: Arc<PromptSession>,
    queued_inputs: Arc<AsyncMutex<VecDeque<String>>>,
    busy_flag: Arc<AtomicBool>,
    exit_requested: Arc<AtomicBool>,
    active_abort: Arc<AsyncMutex<Option<CliActiveAbortHandle>>>,
}

enum CliUiActionOutcome {
    Continue,
    Break,
}

include!("run/ui_actions.rs");

include!("run/frontend_state.rs");

include!("run/session_projection.rs");
include!("run/sse.rs");

#[derive(Debug, Clone)]
struct CliRecoveryAction {
    key: &'static str,
    label: String,
    description: String,
    prompt: String,
}

fn cli_recovery_actions(runtime: &CliExecutionRuntime) -> Vec<CliRecoveryAction> {
    let Some(base_prompt) = runtime.recovery_base_prompt.as_deref() else {
        return Vec::new();
    };

    let mut actions = vec![
        CliRecoveryAction {
            key: "retry",
            label: "Retry last run".to_string(),
            description: "Re-run the last request with the same mode and constraints.".to_string(),
            prompt: format!(
                "Recovery protocol: retry the previous request with the same mode and constraints.\nPreserve any valid prior work, but re-run the task end-to-end where needed.\n\nOriginal request:\n{}",
                base_prompt
            ),
        },
        CliRecoveryAction {
            key: "resume",
            label: "Resume from latest boundary".to_string(),
            description: "Continue from the latest incomplete boundary without restarting discovery.".to_string(),
            prompt: format!(
                "Recovery protocol: resume from the latest incomplete boundary.\nDo not restart discovery from scratch. Preserve prior verified work, artifacts, decisions, and constraints.\n\nOriginal request:\n{}",
                base_prompt
            ),
        },
    ];

    if let Some((stage_label, stage_summary)) = cli_latest_recovery_stage(runtime) {
        actions.push(CliRecoveryAction {
            key: "restart-stage",
            label: format!("Restart stage · {}", stage_label),
            description: "Re-enter this stage as a fresh boundary and recompute downstream work.".to_string(),
            prompt: format!(
                "Recovery protocol: restart scheduler stage `{}`.\nRe-enter this stage as a fresh boundary. Preserve global constraints and prior validated upstream context, but allow this stage and all downstream work to be recomputed from here.\n\nPrevious stage outcome:\n{}\n\nOriginal request:\n{}",
                stage_label, stage_summary, base_prompt
            ),
        });
        actions.push(CliRecoveryAction {
            key: "partial-replay",
            label: format!("Partial replay · {}", stage_label),
            description: "Replay only from this stage boundary and preserve valid prior work.".to_string(),
            prompt: format!(
                "Recovery protocol: partial replay from scheduler stage `{}`.\nRestart from this stage boundary only. Preserve all prior valid work and replay only the downstream work required after this stage.\n\nPrevious stage outcome:\n{}\n\nOriginal request:\n{}",
                stage_label, stage_summary, base_prompt
            ),
        });
    }

    actions
}

fn cli_latest_recovery_stage(runtime: &CliExecutionRuntime) -> Option<(String, String)> {
    let topology = runtime.observed_topology.lock().ok()?;
    let stage_id = topology.stage_order.last()?;
    let stage = topology.nodes.get(stage_id)?;
    let summary = stage
        .recent_event
        .clone()
        .or_else(|| stage.waiting_on.clone())
        .unwrap_or_else(|| stage.status.clone());
    Some((stage.label.clone(), summary))
}

fn cli_print_recovery_actions(runtime: &CliExecutionRuntime) {
    let style = CliStyle::detect();
    let actions = cli_recovery_actions(runtime);
    if actions.is_empty() {
        let lines = vec![
            "No recovery actions available".to_string(),
            style.dim("Send a prompt first, then use /recover"),
        ];
        let _ = print_cli_list_on_surface(Some(runtime), "Recovery Actions", None, &lines, &style);
        return;
    }
    let mut lines = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        lines.push(format!(
            "{}  {} {}",
            style.bold(&format!("{}.", index + 1)),
            action.label,
            style.dim(&format!("[{}]", action.key)),
        ));
        lines.push(format!("   {}", style.dim(&action.description)));
    }
    let _ = print_cli_list_on_surface(
        Some(runtime),
        "Recovery Actions",
        Some("Use /recover <number|key> to execute"),
        &lines,
        &style,
    );
}

fn cli_select_recovery_action(
    runtime: &CliExecutionRuntime,
    selector: &str,
) -> Option<CliRecoveryAction> {
    let actions = cli_recovery_actions(runtime);
    let normalized = selector.trim().to_ascii_lowercase().replace('_', "-");
    if let Ok(index) = normalized.parse::<usize>() {
        return actions.get(index.saturating_sub(1)).cloned();
    }
    actions.into_iter().find(|action| action.key == normalized)
}

fn print_block(
    runtime: Option<&CliExecutionRuntime>,
    block: OutputBlock,
    style: &CliStyle,
) -> anyhow::Result<()> {
    print_block_on_surface(
        runtime.and_then(|runtime| runtime.terminal_surface.as_deref()),
        block,
        style,
    )
}

fn print_rendered(surface: Option<&CliTerminalSurface>, rendered: &str) -> anyhow::Result<()> {
    if rendered.is_empty() {
        return Ok(());
    }
    if let Some(surface) = surface {
        surface.print_text(&rendered)?;
    } else {
        print!("{rendered}");
        io::stdout().flush()?;
    }
    Ok(())
}

fn print_block_on_surface(
    surface: Option<&CliTerminalSurface>,
    block: OutputBlock,
    style: &CliStyle,
) -> anyhow::Result<()> {
    if let Some(surface) = surface {
        surface.print_block(block)?;
    } else {
        print!("{}", render_cli_block_rich(&block, style));
        io::stdout().flush()?;
    }
    Ok(())
}
include!("run/interaction.rs");

// ── CLI agent task handlers ──────────────────────────────────────────

fn cli_list_tasks(runtime: Option<&CliExecutionRuntime>) {
    let style = CliStyle::detect();
    let tasks = global_task_registry().list();
    if tasks.is_empty() {
        let _ = print_cli_list_on_surface(
            runtime,
            "Agent Tasks",
            None,
            &[style.dim("No agent tasks.")],
            &style,
        );
        return;
    }
    let now = chrono::Utc::now().timestamp();
    let mut lines = Vec::new();
    let mut running = 0usize;
    let mut done = 0usize;
    for task in &tasks {
        let (icon, status_str) = match &task.status {
            AgentTaskStatus::Pending => ("◯", "pending".to_string()),
            AgentTaskStatus::Running { step } => {
                running += 1;
                let steps = task
                    .max_steps
                    .map(|m| format!("{}/{}", step, m))
                    .unwrap_or(format!("{}/？", step));
                ("◐", format!("running  {}", steps))
            }
            AgentTaskStatus::Completed { steps } => {
                done += 1;
                ("●", format!("done     {}", steps))
            }
            AgentTaskStatus::Cancelled => {
                done += 1;
                ("✗", "cancelled".to_string())
            }
            AgentTaskStatus::Failed { .. } => {
                done += 1;
                ("✗", "failed".to_string())
            }
        };
        let elapsed = now - task.started_at;
        let elapsed_str = if elapsed < 60 {
            format!("{}s ago", elapsed)
        } else {
            format!("{}m ago", elapsed / 60)
        };
        lines.push(format!(
            "{}  {}  {:<20} {:<16} {}",
            icon, task.id, task.agent_name, status_str, elapsed_str
        ));
    }
    let footer = format!("{} running, {} finished", running, done);
    let _ = print_cli_list_on_surface(runtime, "Agent Tasks", Some(&footer), &lines, &style);
}

fn cli_show_task(id: &str, runtime: Option<&CliExecutionRuntime>) {
    let style = CliStyle::detect();
    match global_task_registry().get(id) {
        Some(task) => {
            let (status_label, step_info) = match &task.status {
                AgentTaskStatus::Pending => ("pending".to_string(), String::new()),
                AgentTaskStatus::Running { step } => {
                    let steps = task
                        .max_steps
                        .map(|m| format!(" (step {}/{})", step, m))
                        .unwrap_or(format!(" (step {}/?)", step));
                    ("running".to_string(), steps)
                }
                AgentTaskStatus::Completed { steps } => {
                    ("completed".to_string(), format!(" ({} steps)", steps))
                }
                AgentTaskStatus::Cancelled => ("cancelled".to_string(), String::new()),
                AgentTaskStatus::Failed { error } => (format!("failed: {}", error), String::new()),
            };
            let now = chrono::Utc::now().timestamp();
            let elapsed = now - task.started_at;
            let elapsed_str = if elapsed < 60 {
                format!("{}s ago", elapsed)
            } else {
                format!("{}m ago", elapsed / 60)
            };
            let mut lines = vec![
                format!("{} {}{}", style.bold("Status:"), status_label, step_info),
                format!("{} {}", style.bold("Started:"), elapsed_str),
                format!("{} {}", style.bold("Prompt:"), task.prompt),
            ];
            if !task.output_tail.is_empty() {
                lines.push(String::new());
                lines.push(style.bold("Recent output:"));
                for line in &task.output_tail {
                    lines.push(format!("  {}", line));
                }
            }
            let title = format!("Task {} — {}", task.id, task.agent_name);
            let _ = print_cli_list_on_surface(runtime, &title, None, &lines, &style);
        }
        None => {
            let lines = vec![format!("Task \"{}\" not found", id)];
            let _ = print_cli_list_on_surface(runtime, "Task Detail", None, &lines, &style);
        }
    }
}

fn cli_kill_task(id: &str, runtime: Option<&CliExecutionRuntime>) {
    let style = CliStyle::detect();
    match rocode_orchestrator::global_lifecycle().cancel_task(id) {
        Ok(()) => {
            let lines = vec![format!(
                "{} Task {} cancelled",
                style.bold_green(style.check()),
                id
            )];
            let _ = print_cli_list_on_surface(runtime, "Task Cancel", None, &lines, &style);
        }
        Err(err) => {
            let lines = vec![format!("{} {}", style.bold_red(style.cross()), err)];
            let _ = print_cli_list_on_surface(runtime, "Task Cancel", None, &lines, &style);
        }
    }
}

// ── CLI session listing ─────────────────────────────────────────────

async fn cli_list_sessions(runtime: Option<&CliExecutionRuntime>) {
    let style = CliStyle::detect();

    let db = match rocode_storage::Database::new().await {
        Ok(db) => db,
        Err(e) => {
            let lines = vec![format!("Failed to open session database: {}", e)];
            let _ = print_cli_list_on_surface(runtime, "Sessions", None, &lines, &style);
            return;
        }
    };

    let session_repo = rocode_storage::SessionRepository::new(db.pool().clone());

    let sessions = match session_repo.list(None, 20).await {
        Ok(sessions) => sessions,
        Err(e) => {
            let lines = vec![format!("Failed to list sessions: {}", e)];
            let _ = print_cli_list_on_surface(runtime, "Sessions", None, &lines, &style);
            return;
        }
    };

    let lines: Vec<String> = if sessions.is_empty() {
        vec![style.dim("No sessions found.")]
    } else {
        sessions
            .iter()
            .map(|session| {
                let title = if session.title.is_empty() {
                    "(untitled)"
                } else {
                    &session.title
                };
                let id_short = if session.id.len() > 8 {
                    &session.id[..8]
                } else {
                    &session.id
                };
                let time_str = format_session_time(session.time.updated);
                format!("{} {} {}", style.dim(id_short), title, style.dim(&time_str))
            })
            .collect()
    };

    let _ = print_cli_list_on_surface(
        runtime,
        "Recent Sessions",
        Some("Use --continue to resume a previous session."),
        &lines,
        &style,
    );
}

fn format_session_time(timestamp: i64) -> String {
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

#[cfg(test)]
mod tests {
    use super::{
        cli_cycle_child_session, cli_focus_child_session, cli_focus_root_session,
        cli_normalize_model_ref, cli_observe_terminal_stream_block, cli_prompt_agent_override,
        cli_prompt_assist_view, cli_prompt_screen_lines, cli_recent_session_info_for_directory,
        cli_render_retained_layout, cli_render_startup_banner, cli_resolve_registry_ui_action,
        cli_resolve_show_thinking, cli_session_update_requires_refresh,
        cli_set_root_server_session, cli_should_emit_scheduler_stage_block, CliExecutionRuntime,
        CliFrontendPhase, CliFrontendProjection, CliLastTurnTokenStats,
        CliObservedExecutionTopology, CliPromptCatalog, CliPromptSelectionState,
        CliRecentSessionInfo, CliRetainedTranscript, CliSessionTokenStats, PermissionMemory,
        TerminalStreamAccumulator,
    };
    use crate::api_client::SessionListItem;
    use crate::api_client::{SessionListHints, SessionListTime};
    use chrono::Utc;
    use rocode_command::cli_style::CliStyle;
    use rocode_command::output_blocks::{MessageBlock, OutputBlock, SchedulerStageBlock};
    use rocode_command::{CommandRegistry, ResolvedUiCommand, UiActionId, UiCommandArgumentKind};
    use rocode_config::{Config, UiPreferencesConfig};
    use std::collections::{BTreeSet, HashMap, VecDeque};
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Mutex as AsyncMutex;

    use rocode_command::cli_spinner::SpinnerGuard;
    use rocode_command::output_blocks::MessageRole as OutputMessageRole;

    #[test]
    fn cli_prompt_omits_agent_when_scheduler_profile_is_active() {
        assert_eq!(cli_prompt_agent_override("build", Some("atlas")), None);
        assert_eq!(
            cli_prompt_agent_override("build", None),
            Some("build".to_string())
        );
    }

    #[test]
    fn cli_show_thinking_defaults_to_hidden_in_cli() {
        assert!(!cli_resolve_show_thinking(false, None, false));
        assert!(!cli_resolve_show_thinking(
            false,
            Some(&Config {
                ui_preferences: Some(UiPreferencesConfig {
                    show_thinking: Some(false),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            false,
        ));
        assert!(cli_resolve_show_thinking(
            false,
            Some(&Config {
                ui_preferences: Some(UiPreferencesConfig {
                    show_thinking: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            false,
        ));
        assert!(cli_resolve_show_thinking(true, None, false));
    }

    fn stage_with_status(status: &str) -> SchedulerStageBlock {
        SchedulerStageBlock {
            stage_id: None,
            profile: Some("prometheus".to_string()),
            stage: "route".to_string(),
            title: "Prometheus · Route".to_string(),
            text: String::new(),
            stage_index: Some(1),
            stage_total: Some(5),
            step: None,
            status: Some(status.to_string()),
            focus: None,
            last_event: None,
            waiting_on: None,
            estimated_context_tokens: None,
            skill_tree_budget: None,
            skill_tree_truncation_strategy: None,
            skill_tree_truncated: None,
            retry_attempt: None,
            activity: None,
            loop_budget: None,
            available_skill_count: None,
            available_agent_count: None,
            available_category_count: None,
            active_skills: Vec::new(),
            active_agents: Vec::new(),
            active_categories: Vec::new(),
            done_agent_count: 0,
            total_agent_count: 0,
            prompt_tokens: None,
            context_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            decision: None,
            child_session_id: None,
        }
    }

    fn test_runtime_with_child_focus_data() -> CliExecutionRuntime {
        let mut root_transcript = CliRetainedTranscript::default();
        root_transcript.append_rendered("● root line\n");

        let mut child_transcript = CliRetainedTranscript::default();
        child_transcript.append_rendered("● child line\n");

        CliExecutionRuntime {
            resolved_agent_name: "build".to_string(),
            resolved_scheduler_profile_name: None,
            resolved_model_label: "openai/gpt-4.1".to_string(),
            working_dir: std::path::PathBuf::from("/tmp/project"),
            observed_topology: Arc::new(Mutex::new(CliObservedExecutionTopology::default())),
            frontend_projection: Arc::new(Mutex::new(CliFrontendProjection {
                transcript: root_transcript.clone(),
                ..Default::default()
            })),
            scheduler_stage_snapshots: Arc::new(Mutex::new(HashMap::new())),
            terminal_surface: None,
            prompt_chrome: None,
            prompt_session: None,
            prompt_session_slot: Arc::new(std::sync::Mutex::new(None)),
            queued_inputs: Arc::new(AsyncMutex::new(VecDeque::new())),
            busy_flag: Arc::new(AtomicBool::new(false)),
            exit_requested: Arc::new(AtomicBool::new(false)),
            active_abort: Arc::new(AsyncMutex::new(None)),
            recovery_base_prompt: None,
            spinner_guard: Arc::new(std::sync::Mutex::new(SpinnerGuard::noop())),
            api_client: None,
            server_session_id: Some("root-session".to_string()),
            related_session_ids: Arc::new(Mutex::new(BTreeSet::from([
                "root-session".to_string(),
                "child-session-a".to_string(),
            ]))),
            root_session_transcript: Arc::new(Mutex::new(root_transcript)),
            child_session_transcripts: Arc::new(Mutex::new(HashMap::from([(
                "child-session-a".to_string(),
                child_transcript,
            )]))),
            stream_accumulators: Arc::new(Mutex::new(HashMap::new())),
            render_states: Arc::new(Mutex::new(HashMap::new())),
            focused_session_id: Arc::new(Mutex::new(None)),
            permission_memory: Arc::new(AsyncMutex::new(PermissionMemory::new())),
            show_thinking: Arc::new(AtomicBool::new(true)),
        }
    }

    fn test_runtime_with_multiple_child_sessions() -> CliExecutionRuntime {
        let runtime = test_runtime_with_child_focus_data();
        runtime
            .related_session_ids
            .lock()
            .expect("related session ids")
            .insert("child-session-b".to_string());
        runtime
            .child_session_transcripts
            .lock()
            .expect("child transcripts")
            .insert("child-session-b".to_string(), {
                let mut transcript = CliRetainedTranscript::default();
                transcript.append_rendered("● second child line\n");
                transcript
            });
        runtime
    }

    #[test]
    fn cli_root_session_reset_clears_stream_accumulators() {
        let mut runtime = test_runtime_with_child_focus_data();
        runtime
            .stream_accumulators
            .lock()
            .expect("stream accumulators")
            .insert("root-session".to_string(), TerminalStreamAccumulator::new());

        cli_set_root_server_session(&mut runtime, "next-root".to_string());

        let accumulators = runtime
            .stream_accumulators
            .lock()
            .expect("stream accumulators");
        assert!(accumulators.is_empty());
    }

    #[test]
    fn cli_terminal_stream_observer_maps_empty_session_to_root_session() {
        let runtime = test_runtime_with_child_focus_data();

        cli_observe_terminal_stream_block(
            &runtime,
            "",
            Some("assistant-1"),
            &OutputBlock::Message(MessageBlock::full(
                OutputMessageRole::Assistant,
                "root message".to_string(),
            )),
        );

        let accumulators = runtime
            .stream_accumulators
            .lock()
            .expect("stream accumulators");
        let root = accumulators
            .get("root-session")
            .expect("root session accumulator");
        let assistant = root
            .last_assistant_message()
            .expect("assistant message recorded");
        assert_eq!(assistant.id, "assistant-1");
    }

    #[test]
    fn cli_prints_scheduler_stage_snapshots_only_on_change() {
        let snapshots = Arc::new(Mutex::new(HashMap::new()));
        let running = stage_with_status("running");
        let done = stage_with_status("done");

        assert!(cli_should_emit_scheduler_stage_block(&snapshots, &running));
        assert!(!cli_should_emit_scheduler_stage_block(&snapshots, &running));
        assert!(cli_should_emit_scheduler_stage_block(&snapshots, &done));
    }

    #[test]
    fn registry_ui_action_resolves_shared_cli_slash_aliases() {
        let registry = CommandRegistry::new();

        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/share"),
            Some(ResolvedUiCommand {
                action_id: UiActionId::ShareSession,
                argument_kind: UiCommandArgumentKind::None,
                argument: None,
            })
        );
        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/unshare"),
            Some(ResolvedUiCommand {
                action_id: UiActionId::UnshareSession,
                argument_kind: UiCommandArgumentKind::None,
                argument: None,
            })
        );
        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/palette"),
            Some(ResolvedUiCommand {
                action_id: UiActionId::ToggleCommandPalette,
                argument_kind: UiCommandArgumentKind::None,
                argument: None,
            })
        );
        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/copy"),
            Some(ResolvedUiCommand {
                action_id: UiActionId::CopySession,
                argument_kind: UiCommandArgumentKind::None,
                argument: None,
            })
        );
        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/rename demo"),
            None
        );
    }

    #[test]
    fn registry_ui_action_resolves_parameterized_shared_cli_commands() {
        let registry = CommandRegistry::new();

        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/model openai/gpt-5"),
            Some(ResolvedUiCommand {
                action_id: UiActionId::OpenModelList,
                argument_kind: UiCommandArgumentKind::ModelRef,
                argument: Some("openai/gpt-5".to_string()),
            })
        );
        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/agent build"),
            Some(ResolvedUiCommand {
                action_id: UiActionId::OpenAgentList,
                argument_kind: UiCommandArgumentKind::AgentRef,
                argument: Some("build".to_string()),
            })
        );
        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/preset atlas"),
            Some(ResolvedUiCommand {
                action_id: UiActionId::OpenPresetList,
                argument_kind: UiCommandArgumentKind::PresetRef,
                argument: Some("atlas".to_string()),
            })
        );
        assert_eq!(
            cli_resolve_registry_ui_action(&registry, "/session abc123"),
            Some(ResolvedUiCommand {
                action_id: UiActionId::OpenSessionList,
                argument_kind: UiCommandArgumentKind::SessionTarget,
                argument: Some("abc123".to_string()),
            })
        );
    }

    #[test]
    fn normalize_model_ref_accepts_slash_and_colon_forms() {
        assert_eq!(
            cli_normalize_model_ref("openai/gpt-5"),
            "openai/gpt-5".to_string()
        );
        assert_eq!(
            cli_normalize_model_ref("openai:gpt-5"),
            "openai/gpt-5".to_string()
        );
        assert_eq!(
            cli_normalize_model_ref(" zhipuai-coding-plan:GLM-5-Turbo "),
            "zhipuai-coding-plan/GLM-5-Turbo".to_string()
        );
    }

    #[test]
    fn normalize_model_ref_keeps_bare_model_ids_unchanged() {
        assert_eq!(cli_normalize_model_ref("gpt-5"), "gpt-5".to_string());
    }

    #[test]
    fn retained_transcript_merges_partial_lines() {
        let mut transcript = CliRetainedTranscript::default();
        transcript.append_rendered("● hello");
        transcript.append_rendered(" world\n");
        transcript.append_rendered("next line\n");

        assert_eq!(
            transcript.committed_lines,
            vec!["● hello world", "next line"]
        );
        assert!(transcript.open_line.is_empty());
        assert_eq!(transcript.rendered_text(), "● hello world\nnext line\n");
    }

    #[test]
    fn focus_child_session_switches_visible_transcript_but_keeps_root_session() {
        let runtime = test_runtime_with_child_focus_data();

        assert!(cli_focus_child_session(&runtime, "child-session-a").expect("focus child session"));

        let visible = runtime
            .frontend_projection
            .lock()
            .expect("frontend projection")
            .transcript
            .rendered_text();
        assert_eq!(visible, "● child line\n");
        assert_eq!(runtime.server_session_id.as_deref(), Some("root-session"));
        assert_eq!(
            runtime
                .focused_session_id
                .lock()
                .expect("focused session")
                .as_deref(),
            Some("child-session-a")
        );
        assert_eq!(
            runtime
                .frontend_projection
                .lock()
                .expect("frontend projection")
                .view_label
                .as_deref(),
            Some("view child child-se")
        );

        assert!(cli_focus_root_session(&runtime).expect("back to root session"));
        let visible = runtime
            .frontend_projection
            .lock()
            .expect("frontend projection")
            .transcript
            .rendered_text();
        assert_eq!(visible, "● root line\n");
        assert_eq!(
            runtime
                .focused_session_id
                .lock()
                .expect("focused session")
                .as_deref(),
            None
        );
        assert_eq!(
            runtime
                .frontend_projection
                .lock()
                .expect("frontend projection")
                .view_label,
            None
        );
    }

    #[test]
    fn cycle_child_session_moves_forward_and_backward() {
        let runtime = test_runtime_with_multiple_child_sessions();

        let first = cli_cycle_child_session(&runtime, true)
            .expect("cycle next from root")
            .expect("first child");
        assert_eq!(first.0, "child-session-a");
        assert_eq!((first.1, first.2), (1, 2));

        let second = cli_cycle_child_session(&runtime, true)
            .expect("cycle next from first")
            .expect("second child");
        assert_eq!(second.0, "child-session-b");
        assert_eq!((second.1, second.2), (2, 2));

        let previous = cli_cycle_child_session(&runtime, false)
            .expect("cycle prev from second")
            .expect("previous child");
        assert_eq!(previous.0, "child-session-a");
        assert_eq!((previous.1, previous.2), (1, 2));
    }

    #[test]
    fn cli_prompt_screen_lines_are_empty_for_transcript_first_mode() {
        assert!(cli_prompt_screen_lines().is_empty());
    }

    #[test]
    fn prompt_assist_completes_switch_command_names() {
        let catalog = CliPromptCatalog {
            models: vec!["openai/gpt-4.1".to_string()],
            agents: vec!["build".to_string()],
            presets: vec!["prometheus".to_string()],
        };
        let selection = CliPromptSelectionState {
            model: "openai/gpt-4.1".to_string(),
            agent: "build".to_string(),
            preset: Some("prometheus".to_string()),
        };

        let assist = cli_prompt_assist_view(&catalog, &selection, "/mo", 3);

        assert!(assist
            .screen_lines
            .iter()
            .any(|line| line.contains("/model")));
        assert_eq!(
            assist.completion,
            Some(rocode_command::cli_prompt::PromptCompletion {
                line: "/model ".to_string(),
                cursor_pos: 7,
            })
        );
    }

    #[test]
    fn prompt_assist_filters_model_candidates() {
        let catalog = CliPromptCatalog {
            models: vec![
                "ethnopic/test-model-large".to_string(),
                "dashscope/qwen-max".to_string(),
                "dashscope/qwen-plus".to_string(),
            ],
            agents: vec!["build".to_string()],
            presets: vec!["prometheus".to_string()],
        };
        let selection = CliPromptSelectionState {
            model: "dashscope/qwen-plus".to_string(),
            agent: "build".to_string(),
            preset: Some("prometheus".to_string()),
        };

        let assist = cli_prompt_assist_view(&catalog, &selection, "/model qwen", 11);

        assert!(assist
            .screen_lines
            .iter()
            .any(|line| line.contains("dashscope/qwen-max")));
        assert!(assist
            .screen_lines
            .iter()
            .any(|line| line.contains("dashscope/qwen-plus [active]")));
        assert_eq!(
            assist.completion,
            Some(rocode_command::cli_prompt::PromptCompletion {
                line: "/model dashscope/qwen-max".to_string(),
                cursor_pos: 25,
            })
        );
    }

    #[test]
    fn prompt_assist_shows_preset_values_after_exact_command() {
        let catalog = CliPromptCatalog {
            models: vec!["openai/gpt-4.1".to_string()],
            agents: vec!["build".to_string()],
            presets: vec!["atlas".to_string(), "prometheus".to_string()],
        };
        let selection = CliPromptSelectionState {
            model: "openai/gpt-4.1".to_string(),
            agent: "build".to_string(),
            preset: Some("atlas".to_string()),
        };

        let assist = cli_prompt_assist_view(&catalog, &selection, "/preset", 7);

        assert!(assist
            .screen_lines
            .iter()
            .any(|line| line.contains("/preset suggestions")));
        assert_eq!(
            assist.completion,
            Some(rocode_command::cli_prompt::PromptCompletion {
                line: "/preset ".to_string(),
                cursor_pos: 8,
            })
        );
    }

    #[test]
    fn startup_banner_uses_recent_session_metadata() {
        let now = Utc::now().timestamp_millis();
        let sessions = vec![SessionListItem {
            id: "s1".to_string(),
            slug: "s1".to_string(),
            project_id: "p1".to_string(),
            directory: "/tmp/project".to_string(),
            parent_id: None,
            title: "Research Session".to_string(),
            version: "v1".to_string(),
            time: SessionListTime {
                created: now,
                updated: now,
                compacting: None,
                archived: None,
            },
            summary: None,
            hints: Some(SessionListHints {
                current_model: None,
                model_provider: Some("zhipuai".to_string()),
                model_id: Some("GLM-5".to_string()),
                scheduler_profile: Some("prometheus".to_string()),
                resolved_scheduler_profile: None,
                agent: None,
            }),
            pending_command_invocation: None,
        }];
        let info = cli_recent_session_info_for_directory(&sessions, Path::new("/tmp/project"))
            .expect("recent session info");
        assert_eq!(
            info,
            CliRecentSessionInfo {
                title: Some("Research Session".to_string()),
                model_label: Some("zhipuai/GLM-5".to_string()),
                preset_label: Some("prometheus".to_string()),
            }
        );

        let banner = cli_render_startup_banner(&CliStyle::plain(), Some(&info));
        assert!(banner.contains("ROCode"));
        assert!(banner.contains("Research Session"));
        assert!(banner.contains("zhipuai/GLM-5"));
        assert!(banner.contains("prometheus"));
    }

    #[test]
    fn retained_layout_emits_session_messages_sidebar_and_active_boxes() {
        let style = CliStyle::plain();
        let mut projection = CliFrontendProjection {
            phase: CliFrontendPhase::Busy,
            active_label: Some("assistant response".to_string()),
            view_label: Some("view child child-abc".to_string()),
            queue_len: 2,
            active_stage: Some(stage_with_status("running")),
            session_runtime: None,
            stage_summaries: Vec::new(),
            telemetry_topology: None,
            events_browser: None,
            transcript: CliRetainedTranscript::default(),
            sidebar_collapsed: false,
            active_collapsed: false,
            session_title: Some("Test Session".to_string()),
            current_model_label: Some("openai/gpt-4.1".to_string()),
            scroll_offset: 0,
            token_stats: CliSessionTokenStats::default(),
            last_turn_tokens: CliLastTurnTokenStats::default(),
            model_catalog: HashMap::new(),
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
        };
        projection
            .transcript
            .append_rendered("● user prompt\n\n● assistant reply\n");
        let topology = CliObservedExecutionTopology {
            active: true,
            ..Default::default()
        };

        let lines = cli_render_retained_layout(
            "Preset prometheus",
            "Model auto",
            "~/tests/rust/rocode",
            &projection,
            &topology,
            &style,
        );
        let joined = lines.join("\n");

        assert!(joined.contains("ROCode"));
        assert!(joined.contains("Messages"));
        assert!(joined.contains("Sidebar"));
        assert!(joined.contains("Active"));
        assert!(joined.contains("assistant reply"));
        assert!(joined.contains("Test Session"));
        assert!(joined.contains("view child child-abc"));
    }

    #[test]
    fn retained_layout_collapses_sidebar() {
        let style = CliStyle::plain();
        let projection = CliFrontendProjection {
            phase: CliFrontendPhase::Idle,
            sidebar_collapsed: true,
            active_collapsed: false,
            session_title: Some("Collapsed Test".to_string()),
            ..Default::default()
        };
        let topology = CliObservedExecutionTopology::default();

        let lines = cli_render_retained_layout(
            "Agent build",
            "Model auto",
            "~/workspace",
            &projection,
            &topology,
            &style,
        );
        let joined = lines.join("\n");

        assert!(joined.contains("ROCode"));
        assert!(joined.contains("Messages"));
        assert!(!joined.contains("╭ Sidebar"));
        assert!(joined.contains("Active"));
    }

    #[test]
    fn footer_text_surfaces_child_focus_state() {
        let projection = CliFrontendProjection {
            phase: CliFrontendPhase::Busy,
            view_label: Some("view child abcd1234".to_string()),
            ..Default::default()
        };

        let footer = projection.footer_text();

        assert!(footer.contains("Busy"));
        assert!(footer.contains("view child abcd1234"));
        assert!(footer.contains("/child"));
    }

    #[test]
    fn retained_layout_collapses_active() {
        let style = CliStyle::plain();
        let projection = CliFrontendProjection {
            phase: CliFrontendPhase::Idle,
            sidebar_collapsed: false,
            active_collapsed: true,
            session_title: None,
            ..Default::default()
        };
        let topology = CliObservedExecutionTopology::default();

        let lines = cli_render_retained_layout(
            "Agent build",
            "Model auto",
            "~/workspace",
            &projection,
            &topology,
            &style,
        );
        let joined = lines.join("\n");

        assert!(joined.contains("Sidebar"));
        assert!(joined.contains("Active"));
        assert!(joined.contains("/active to expand"));
    }

    #[test]
    fn retained_layout_active_panel_adapts_to_content() {
        let style = CliStyle::plain();
        let topology = CliObservedExecutionTopology::default();
        let minimal_stage = stage_with_status("running");

        let proj_minimal = CliFrontendProjection {
            phase: CliFrontendPhase::Busy,
            active_stage: Some(minimal_stage),
            sidebar_collapsed: true,
            active_collapsed: false,
            session_title: Some("Test".to_string()),
            ..Default::default()
        };
        let lines_minimal = cli_render_retained_layout(
            "Agent build",
            "Model auto",
            "~/test",
            &proj_minimal,
            &topology,
            &style,
        );

        let mut rich_stage = stage_with_status("running");
        rich_stage.focus = Some("analyzing codebase".to_string());
        rich_stage.last_event = Some("tool_call: read_file".to_string());
        rich_stage.activity = Some("Reviewing architecture".to_string());
        rich_stage.available_skill_count = Some(12);
        rich_stage.available_agent_count = Some(4);
        rich_stage.active_skills = vec!["planner".to_string(), "reviewer".to_string()];
        rich_stage.total_agent_count = 3;
        rich_stage.done_agent_count = 1;
        rich_stage.child_session_id = Some("child-abc".to_string());

        let proj_rich = CliFrontendProjection {
            phase: CliFrontendPhase::Busy,
            active_stage: Some(rich_stage),
            sidebar_collapsed: true,
            active_collapsed: false,
            session_title: Some("Test".to_string()),
            ..Default::default()
        };
        let lines_rich = cli_render_retained_layout(
            "Agent build",
            "Model auto",
            "~/test",
            &proj_rich,
            &topology,
            &style,
        );

        assert!(
            lines_rich.len() > lines_minimal.len(),
            "Rich active panel ({} lines) should be taller than minimal ({} lines)",
            lines_rich.len(),
            lines_minimal.len(),
        );

        let joined_rich = lines_rich.join("\n");
        assert!(joined_rich.contains("Active"));
        assert!(joined_rich.contains("child-abc"));
        assert!(joined_rich.contains("planner"));
    }

    #[test]
    fn session_updated_refresh_allowlist_is_explicit() {
        assert!(cli_session_update_requires_refresh(Some("prompt.final")));
        assert!(cli_session_update_requires_refresh(Some("stream.final")));
        assert!(cli_session_update_requires_refresh(Some(
            "prompt.completed"
        )));
        assert!(cli_session_update_requires_refresh(Some(
            "session.title.set"
        )));
        assert!(!cli_session_update_requires_refresh(Some(
            "prompt.scheduler.stage.content"
        )));
        assert!(!cli_session_update_requires_refresh(Some("prompt.stream")));
        assert!(!cli_session_update_requires_refresh(None));
    }
}
