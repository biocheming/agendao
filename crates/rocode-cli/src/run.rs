use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rocode_agent::{AgentInfo, AgentRegistry};
#[cfg(test)]
use rocode_command::cli_panel::{truncate_display, wrap_display_text};
use rocode_command::cli_permission::{prompt_permission, PermissionDecision};
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
    render_cli_block_rich, BlockTone, MessageBlock, MessagePhase, MessageRole as OutputMessageRole,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CliPromptAuxLane {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliPromptAuxLine {
    pub(super) lane: CliPromptAuxLane,
    pub(super) text: String,
}

pub(super) fn cli_prompt_aux_line(block: &OutputBlock) -> Option<CliPromptAuxLine> {
    match block {
        OutputBlock::QueueItem(item) => Some(CliPromptAuxLine {
            lane: CliPromptAuxLane::Info,
            text: format!("Queued #{}: {}", item.position, item.text),
        }),
        OutputBlock::Status(status) => {
            let (lane, prefix) = match status.tone {
                BlockTone::Title => (CliPromptAuxLane::Info, "Info"),
                BlockTone::Normal => (CliPromptAuxLane::Info, "Status"),
                BlockTone::Muted => (CliPromptAuxLane::Info, "Status"),
                BlockTone::Success => (CliPromptAuxLane::Info, "Done"),
                BlockTone::Warning => (CliPromptAuxLane::Warning, "Warning"),
                BlockTone::Error => (CliPromptAuxLane::Error, "Error"),
            };
            Some(CliPromptAuxLine {
                lane,
                text: cli_prompt_aux_text(prefix, &status.text),
            })
        }
        _ => None,
    }
}

fn cli_prompt_aux_text(prefix: &str, text: &str) -> String {
    let trimmed = text.trim();
    let Some(head) = trimmed.get(..prefix.len()) else {
        return format!("{prefix}: {trimmed}");
    };
    if head.eq_ignore_ascii_case(prefix) {
        let rest = trimmed[prefix.len()..].trim_start();
        if rest.is_empty() || rest.starts_with(':') || rest.starts_with('.') {
            return trimmed.to_string();
        }
    }
    format!("{prefix}: {trimmed}")
}

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
            port,
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
    scheduler_profile_name: Option<String>,
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
    /// Root session plus any explicitly attached sessions for the active execution tree.
    related_session_ids: Arc<Mutex<BTreeSet<String>>>,
    /// Persisted root-session transcript rebuilt from session history / final state.
    root_history_transcript: Arc<Mutex<CliVisibleTranscript>>,
    /// Canonical visible transcript snapshot for the root session even when the
    /// operator temporarily focuses an attached-session view.
    root_session_transcript: Arc<Mutex<CliVisibleTranscript>>,
    /// Background transcripts for non-root attached sessions. These are populated
    /// from the unified event surface but not rendered into the main transcript
    /// until the operator explicitly focuses one.
    attached_session_transcripts: Arc<Mutex<HashMap<String, CliVisibleTranscript>>>,
    stream_accumulators: Arc<Mutex<HashMap<String, TerminalStreamAccumulator>>>,
    render_states: Arc<Mutex<HashMap<String, TerminalSemanticStreamRenderState>>>,
    active_tool_labels: Arc<Mutex<HashMap<String, String>>>,
    /// Local CLI-only focus target. `None` means the root session remains visible.
    focused_session_id: Arc<Mutex<Option<String>>>,
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

#[path = "run/frontend_state_projection.rs"]
mod frontend_state_projection;
#[path = "run/frontend_state_prompt.rs"]
mod frontend_state_prompt;
#[path = "run/frontend_state_surface.rs"]
mod frontend_state_surface;
#[path = "run/frontend_state_topology.rs"]
mod frontend_state_topology;
#[path = "run/frontend_state_types.rs"]
pub(crate) mod frontend_state_types;
use frontend_state_projection::CliFrontendPhase;
use frontend_state_prompt::CliPromptChrome;
use frontend_state_surface::{
    cli_copy_target_transcript, cli_refresh_prompt, print_cli_list_on_surface, CliTerminalSurface,
};
use frontend_state_topology::{cli_print_execution_topology, CliObservedExecutionTopology};
include!("run/frontend_state.rs");

#[path = "run/session_projection_events.rs"]
mod session_projection_events;
#[path = "run/session_projection_insights.rs"]
mod session_projection_insights;
#[path = "run/session_projection_layout.rs"]
mod session_projection_layout;
#[path = "run/session_projection_usage.rs"]
mod session_projection_usage;
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
    use super::frontend_state_prompt::{
        cli_prompt_assist_view, cli_prompt_lane_screen_lines_from_projection,
        cli_prompt_screen_lines_with_budget, CliPromptCatalog, CliPromptSelectionState,
    };
    use super::frontend_state_types::CliLastTurnTokenStats;
    use super::{
        cli_cycle_attached_session, cli_focus_attached_session, cli_focus_root_session,
        cli_normalize_model_ref, cli_observe_terminal_stream_block, cli_prompt_agent_override,
        cli_prompt_aux_line, cli_push_runtime_aux_block, cli_recent_session_info_for_directory,
        cli_render_live_slot_snapshot, cli_render_retained_layout, cli_render_session_block,
        cli_render_startup_banner, cli_replace_root_history_transcript,
        cli_resolve_registry_ui_action, cli_resolve_show_thinking, cli_restore_compact_summary,
        cli_session_update_requires_refresh, cli_set_root_server_session,
        cli_should_emit_scheduler_stage_block, cli_sync_root_history_to_visible, handle_sse_event,
        CliExecutionRuntime, CliFrontendPhase, CliFrontendProjection, CliObservedExecutionTopology,
        CliPromptAuxLane, CliRecentSessionInfo, CliServerEvent, CliSessionTokenStats,
        CliVisibleTranscript, TerminalStreamAccumulator,
    };
    use crate::api_client::SessionListItem;
    use crate::api_client::{SessionListHints, SessionListTime};
    use chrono::Utc;
    use rocode_command::cli_style::CliStyle;
    use rocode_command::governance_fixtures::live_transcript_state_fixture;
    use rocode_command::output_blocks::{
        MessageBlock, OutputBlock, SchedulerStageBlock, StatusBlock,
    };
    use rocode_command::terminal_presentation::TerminalMessageRole;
    use rocode_command::{CommandRegistry, ResolvedUiCommand, UiActionId, UiCommandArgumentKind};
    use rocode_config::{Config, UiPreferencesConfig};
    use rocode_util::util::color::strip_ansi;
    use std::collections::{BTreeSet, HashMap, VecDeque};
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Mutex as AsyncMutex;

    use rocode_command::cli_spinner::SpinnerGuard;
    use rocode_command::output_blocks::MessageRole as OutputMessageRole;
    use rocode_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};

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
            cache_miss_tokens: None,
            cache_write_tokens: None,
            decision: None,
            attached_session_id: None,
        }
    }

    fn test_runtime_with_attached_focus_data() -> CliExecutionRuntime {
        let mut root_transcript = CliVisibleTranscript::default();
        root_transcript.append_rendered("● root line\n");

        let mut attached_transcript = CliVisibleTranscript::default();
        attached_transcript.append_rendered("● attached line\n");

        let mut visible_transcript = CliVisibleTranscript::default();
        visible_transcript.append_rendered("● root line\n");

        CliExecutionRuntime {
            resolved_agent_name: "build".to_string(),
            scheduler_profile_name: None,
            resolved_model_label: "openai/gpt-4.1".to_string(),
            working_dir: std::path::PathBuf::from("/tmp/project"),
            observed_topology: Arc::new(Mutex::new(CliObservedExecutionTopology::default())),
            frontend_projection: Arc::new(Mutex::new(CliFrontendProjection {
                transcript: visible_transcript,
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
                "attached-session-a".to_string(),
            ]))),
            root_history_transcript: Arc::new(Mutex::new({
                let mut transcript = CliVisibleTranscript::default();
                transcript.append_rendered("● root line\n");
                transcript
            })),
            root_session_transcript: Arc::new(Mutex::new(root_transcript)),
            attached_session_transcripts: Arc::new(Mutex::new(HashMap::from([(
                "attached-session-a".to_string(),
                attached_transcript,
            )]))),
            stream_accumulators: Arc::new(Mutex::new(HashMap::new())),
            render_states: Arc::new(Mutex::new(HashMap::new())),
            active_tool_labels: Arc::new(Mutex::new(HashMap::new())),
            focused_session_id: Arc::new(Mutex::new(None)),
            show_thinking: Arc::new(AtomicBool::new(true)),
        }
    }

    fn test_runtime_with_multiple_attached_sessions() -> CliExecutionRuntime {
        let runtime = test_runtime_with_attached_focus_data();
        runtime
            .related_session_ids
            .lock()
            .expect("related session ids")
            .insert("attached-session-b".to_string());
        runtime
            .attached_session_transcripts
            .lock()
            .expect("attached transcripts")
            .insert("attached-session-b".to_string(), {
                let mut transcript = CliVisibleTranscript::default();
                transcript.append_rendered("● second attached line\n");
                transcript
            });
        runtime
    }

    fn live_identity(
        message_id: &str,
        part_key: &str,
        part_kind: LiveMessagePartKind,
        phase: LivePartPhase,
        wire_legacy_block_id: Option<&str>,
    ) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: message_id.to_string(),
            part_key: part_key.to_string(),
            part_kind,
            phase,
            legacy_block_id: wire_legacy_block_id.map(str::to_string),
        }
    }

    fn output_block_event(
        id: Option<&str>,
        live_identity: Option<LiveMessagePartIdentity>,
        payload: serde_json::Value,
    ) -> CliServerEvent {
        CliServerEvent::OutputBlock {
            session_id: "root-session".to_string(),
            id: id.map(str::to_string),
            live_identity,
            payload,
        }
    }

    #[test]
    fn cli_root_session_reset_clears_stream_accumulators() {
        let mut runtime = test_runtime_with_attached_focus_data();
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
        let runtime = test_runtime_with_attached_focus_data();

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
            .messages()
            .iter()
            .rev()
            .find(|message| matches!(message.role, TerminalMessageRole::Assistant))
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
        let mut transcript = CliVisibleTranscript::default();
        transcript.append_rendered("● hello");
        transcript.append_rendered(" world\n");
        transcript.append_rendered("next line\n");

        assert_eq!(transcript.rendered_text(), "● hello world\nnext line\n");
    }

    #[test]
    fn interactive_live_assistant_stream_keeps_single_header_across_tool_cycles_and_full_chunks() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();
        let reasoning_identity = live_identity(
            "assistant-final",
            rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
            LiveMessagePartKind::AssistantReasoning,
            LivePartPhase::Snapshot,
            Some("assistant-final"),
        );
        let tool_start_identity = live_identity(
            "assistant-final",
            &rocode_types::tool_call_part_key("tool-1"),
            LiveMessagePartKind::ToolCall,
            LivePartPhase::Start,
            Some("tool-1"),
        );
        let tool_done_identity = live_identity(
            "assistant-final",
            &rocode_types::tool_result_part_key("tool-1"),
            LiveMessagePartKind::ToolResult,
            LivePartPhase::End,
            Some("tool-1"),
        );

        for event in [
            output_block_event(
                Some("assistant-final"),
                Some(reasoning_identity.clone()),
                serde_json::json!({
                    "kind": "reasoning",
                    "phase": "full",
                    "text": "Now I have enough information."
                }),
            ),
            output_block_event(
                Some("tool-1"),
                Some(tool_start_identity),
                serde_json::json!({
                    "kind": "tool",
                    "phase": "start",
                    "name": "websearch",
                    "detail": ""
                }),
            ),
            output_block_event(
                Some("tool-1"),
                Some(tool_done_identity),
                serde_json::json!({
                    "kind": "tool",
                    "phase": "done",
                    "name": "websearch",
                    "detail": "query finished"
                }),
            ),
            output_block_event(
                Some("assistant-final"),
                Some(live_identity(
                    "assistant-final",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Snapshot,
                    Some("assistant-final"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "现在"
                }),
            ),
            output_block_event(
                Some("assistant-final"),
                Some(live_identity(
                    "assistant-final",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Snapshot,
                    Some("assistant-final"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "我已"
                }),
            ),
            output_block_event(
                Some("assistant-final"),
                Some(live_identity(
                    "assistant-final",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Snapshot,
                    Some("assistant-final"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "掌握"
                }),
            ),
            output_block_event(
                Some("assistant-final"),
                Some(live_identity(
                    "assistant-final",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Snapshot,
                    Some("assistant-final"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "现在我已掌握充分信息，以下是完整调研报告。"
                }),
            ),
            output_block_event(
                Some("assistant-final"),
                Some(live_identity(
                    "assistant-final",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::End,
                    Some("assistant-final"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            ),
        ] {
            handle_sse_event(&runtime, event, &style);
        }

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();

        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert!(
            rendered.contains("[message:assistant] 现在我已掌握充分信息，以下是完整调研报告。"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("[message:assistant] 现在[message:assistant]"),
            "assistant output should not restart header inside the same message: {rendered}"
        );
    }

    #[test]
    fn interactive_live_identity_output_block_skips_compat_accumulator_observer() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("assistant-1"),
                Some(live_identity(
                    "assistant-1",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Snapshot,
                    Some("assistant-1"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "hello"
                }),
            ),
            &style,
        );

        let accumulators = runtime
            .stream_accumulators
            .lock()
            .expect("stream accumulators");
        assert!(
            accumulators.get("root-session").is_none(),
            "live-identity events should not hydrate compatibility accumulator"
        );
    }

    #[test]
    fn interactive_reasoning_snapshots_replace_same_slot_without_replaying_header() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        for text in ["Thinking first", "Thinking first second"] {
            handle_sse_event(
                &runtime,
                output_block_event(
                    Some("assistant-1"),
                    Some(live_identity(
                        "assistant-1",
                        rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                        LiveMessagePartKind::AssistantReasoning,
                        LivePartPhase::Snapshot,
                        Some("assistant-1"),
                    )),
                    serde_json::json!({
                        "kind": "reasoning",
                        "phase": "full",
                        "text": text
                    }),
                ),
                &style,
            );
        }

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered.matches("[thinking]").count(), 1, "{rendered}");
        assert!(rendered.contains("Thinking first second"), "{rendered}");
        assert!(
            !rendered.contains("Thinking first\n[thinking]"),
            "{rendered}"
        );
    }

    #[test]
    fn prompt_owned_live_reasoning_events_still_surface_visible_output() {
        let mut runtime = test_runtime_with_attached_focus_data();
        runtime.terminal_surface = Some(Arc::new(
            crate::run::frontend_state_surface::CliTerminalSurface::new(
                CliStyle::plain(),
                runtime.frontend_projection.clone(),
            ),
        ));
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("assistant-1"),
                Some(live_identity(
                    "assistant-1",
                    rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantReasoning,
                    LivePartPhase::Snapshot,
                    Some("assistant-1"),
                )),
                serde_json::json!({
                    "kind": "reasoning",
                    "phase": "full",
                    "text": "Thinking visible"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        assert!(
            !rendered.contains("Thinking visible"),
            "snapshot reasoning should stay out of the visible prompt transcript: {rendered}"
        );
        let root_rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(
            root_rendered.contains("Thinking visible"),
            "{root_rendered}"
        );
        assert!(
            runtime
                .render_states
                .lock()
                .expect("render states")
                .is_empty(),
            "prompt-owned tool-result snapshots should bypass secondary semantic render"
        );
    }

    #[test]
    fn prompt_owned_live_reasoning_refreshes_prompt_surface_without_raw_stream_passthrough() {
        let mut runtime = test_runtime_with_attached_focus_data();
        let surface = Arc::new(crate::run::frontend_state_surface::CliTerminalSurface::new(
            CliStyle::plain(),
            runtime.frontend_projection.clone(),
        ));
        runtime.terminal_surface = Some(surface.clone());
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("assistant-1"),
                Some(live_identity(
                    "assistant-1",
                    rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantReasoning,
                    LivePartPhase::Snapshot,
                    Some("assistant-1"),
                )),
                serde_json::json!({
                    "kind": "reasoning",
                    "phase": "full",
                    "text": "Thinking visible"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        assert!(!rendered.contains("Thinking visible"), "{rendered}");
        assert!(
            !surface.has_prompt_snapshot(),
            "snapshot-only reasoning should not refresh the prompt transcript"
        );
        let root_rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(
            root_rendered.contains("Thinking visible"),
            "{root_rendered}"
        );
        assert_eq!(
            surface.emitted_render_count(),
            0,
            "prompt-owned transcript-bearing reasoning should not write raw stream history"
        );
    }

    #[test]
    fn prompt_owned_live_assistant_refreshes_prompt_surface_without_raw_stream_passthrough() {
        let mut runtime = test_runtime_with_attached_focus_data();
        let surface = Arc::new(crate::run::frontend_state_surface::CliTerminalSurface::new(
            CliStyle::plain(),
            runtime.frontend_projection.clone(),
        ));
        runtime.terminal_surface = Some(surface.clone());
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("assistant-1"),
                Some(live_identity(
                    "assistant-1",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Snapshot,
                    Some("assistant-1"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "Assistant visible"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        assert!(!rendered.contains("Assistant visible"), "{rendered}");
        let root_rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(
            root_rendered.contains("Assistant visible"),
            "{root_rendered}"
        );
        assert_eq!(
            surface.emitted_render_count(),
            0,
            "prompt-owned transcript-bearing assistant text should not write raw stream history"
        );
        assert!(
            runtime
                .render_states
                .lock()
                .expect("render states")
                .is_empty(),
            "prompt-owned transcript-bearing assistant text should bypass secondary semantic render"
        );
    }

    #[test]
    fn prompt_owned_reasoning_rewrite_keeps_latest_slot_without_replaying_history() {
        let mut runtime = test_runtime_with_attached_focus_data();
        runtime.terminal_surface = Some(Arc::new(
            crate::run::frontend_state_surface::CliTerminalSurface::new(
                CliStyle::plain(),
                runtime.frontend_projection.clone(),
            ),
        ));
        let style = CliStyle::plain();

        for text in [".", "categories"] {
            handle_sse_event(
                &runtime,
                output_block_event(
                    Some("assistant-1"),
                    Some(live_identity(
                        "assistant-1",
                        rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                        LiveMessagePartKind::AssistantReasoning,
                        LivePartPhase::Snapshot,
                        Some("assistant-1"),
                    )),
                    serde_json::json!({
                        "kind": "reasoning",
                        "phase": "full",
                        "text": text
                    }),
                ),
                &style,
            );
        }

        let visible_rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        assert_eq!(visible_rendered, "● root line\n", "{visible_rendered}");

        let latest_slot = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(latest_slot.contains("categories"), "{latest_slot}");

        let projection = runtime.frontend_projection.lock().expect("projection");
        assert_eq!(projection.active_label.as_deref(), Some("Thinking"));
        assert!(
            runtime
                .render_states
                .lock()
                .expect("render states")
                .is_empty(),
            "prompt-owned reasoning snapshots should bypass secondary semantic render"
        );
    }

    #[test]
    fn prompt_owned_live_tool_results_still_surface_visible_output() {
        let mut runtime = test_runtime_with_attached_focus_data();
        runtime.terminal_surface = Some(Arc::new(
            crate::run::frontend_state_surface::CliTerminalSurface::new(
                CliStyle::plain(),
                runtime.frontend_projection.clone(),
            ),
        ));
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("tool-1"),
                Some(live_identity(
                    "assistant-1",
                    &rocode_types::tool_result_part_key("tool-1"),
                    LiveMessagePartKind::ToolResult,
                    LivePartPhase::Snapshot,
                    Some("tool-1"),
                )),
                serde_json::json!({
                    "kind": "tool",
                    "phase": "done",
                    "name": "SkillsList",
                    "detail": "11 skills · literature-research/skills"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        assert!(
            rendered.contains("SkillsList") || rendered.contains("11 skills"),
            "prompt-owned live tool results should still emit visible output: {rendered}"
        );
        let root_rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, root_rendered, "{rendered}");
    }

    #[test]
    fn prompt_owned_reasoning_updates_do_not_append_stream_copies_before_tool_output() {
        let mut runtime = test_runtime_with_attached_focus_data();
        runtime.terminal_surface = Some(Arc::new(
            crate::run::frontend_state_surface::CliTerminalSurface::new(
                CliStyle::plain(),
                runtime.frontend_projection.clone(),
            ),
        ));
        let style = CliStyle::plain();

        for text in [".", "categories"] {
            handle_sse_event(
                &runtime,
                output_block_event(
                    Some("assistant-1"),
                    Some(live_identity(
                        "assistant-1",
                        rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                        LiveMessagePartKind::AssistantReasoning,
                        LivePartPhase::Snapshot,
                        Some("assistant-1"),
                    )),
                    serde_json::json!({
                        "kind": "reasoning",
                        "phase": "full",
                        "text": text
                    }),
                ),
                &style,
            );
        }

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("tool-1"),
                Some(live_identity(
                    "assistant-1",
                    &rocode_types::tool_result_part_key("tool-1"),
                    LiveMessagePartKind::ToolResult,
                    LivePartPhase::Snapshot,
                    Some("tool-1"),
                )),
                serde_json::json!({
                    "kind": "tool",
                    "phase": "done",
                    "name": "SkillsCategories",
                    "detail": "4 categories"
                }),
            ),
            &style,
        );

        let projection_rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        let root_rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();

        assert_eq!(projection_rendered, root_rendered, "{projection_rendered}");
        assert_eq!(
            projection_rendered.matches("[thinking]").count(),
            1,
            "{projection_rendered}"
        );
        assert!(
            projection_rendered.contains("categories"),
            "{projection_rendered}"
        );
        assert!(
            projection_rendered.contains("SkillsCategories"),
            "{projection_rendered}"
        );
    }

    #[test]
    fn prompt_owned_empty_reasoning_lifecycle_does_not_materialize_blank_history() {
        let mut runtime = test_runtime_with_attached_focus_data();
        runtime.terminal_surface = Some(Arc::new(
            crate::run::frontend_state_surface::CliTerminalSurface::new(
                CliStyle::plain(),
                runtime.frontend_projection.clone(),
            ),
        ));
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("assistant-1"),
                Some(live_identity(
                    "assistant-1",
                    rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantReasoning,
                    LivePartPhase::Start,
                    Some("assistant-1"),
                )),
                serde_json::json!({
                    "kind": "reasoning",
                    "phase": "start",
                    "text": ""
                }),
            ),
            &style,
        );

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("assistant-1"),
                Some(live_identity(
                    "assistant-1",
                    rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantReasoning,
                    LivePartPhase::End,
                    Some("assistant-1"),
                )),
                serde_json::json!({
                    "kind": "reasoning",
                    "phase": "end",
                    "text": ""
                }),
            ),
            &style,
        );

        let projection_rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        let root_rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();

        assert_eq!(projection_rendered, root_rendered, "{projection_rendered}");
        assert_eq!(
            projection_rendered, "● root line\n",
            "{projection_rendered}"
        );
    }

    #[test]
    fn interactive_scheduler_stage_identity_does_not_enter_live_slot_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("stage-1"),
                Some(live_identity(
                    "assistant-1",
                    &rocode_types::scheduler_stage_part_key("stage-1"),
                    LiveMessagePartKind::SchedulerStage,
                    LivePartPhase::Snapshot,
                    None,
                )),
                serde_json::json!({
                    "kind": "scheduler_stage",
                    "stage": "research",
                    "title": "Research",
                    "text": "planning",
                    "status": "running"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
    }

    #[test]
    fn interactive_tool_call_identity_does_not_enter_root_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("tool-call-1"),
                Some(live_identity(
                    "assistant-1",
                    &rocode_types::tool_call_part_key("tool-call-1"),
                    LiveMessagePartKind::ToolCall,
                    LivePartPhase::Snapshot,
                    Some("tool-call-1"),
                )),
                serde_json::json!({
                    "kind": "tool",
                    "phase": "running",
                    "name": "SkillsList",
                    "detail": ""
                }),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
    }

    #[test]
    fn interactive_missing_identity_tool_running_does_not_enter_root_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("tool-call-1"),
                None,
                serde_json::json!({
                    "kind": "tool",
                    "phase": "running",
                    "name": "SkillsList",
                    "detail": "{\"category\":\"literature-research/skills\"}"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
    }

    #[test]
    fn interactive_missing_identity_scheduler_stage_running_does_not_enter_root_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("stage-1"),
                None,
                serde_json::json!({
                    "kind": "scheduler_stage",
                    "stage": "research",
                    "title": "Research",
                    "text": "planning",
                    "status": "running"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
    }

    #[test]
    fn interactive_missing_identity_scheduler_stage_done_does_not_enter_root_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("stage-1"),
                None,
                serde_json::json!({
                    "kind": "scheduler_stage",
                    "stage": "research",
                    "title": "Research",
                    "text": "planning complete",
                    "status": "done"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
    }

    #[test]
    fn non_focused_root_error_stays_out_of_root_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            CliServerEvent::Error {
                session_id: "root-session".to_string(),
                error: "boom".to_string(),
                message_id: Some("message-1".to_string()),
                done: Some(true),
            },
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
        let run_tail = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .run_tail
            .clone();
        assert_eq!(
            run_tail.as_ref().map(|tail| tail.status.as_str()),
            Some("error")
        );
        assert_eq!(
            run_tail.as_ref().and_then(|tail| tail.detail.as_deref()),
            Some("boom")
        );
    }

    #[test]
    fn non_focused_root_usage_stays_out_of_root_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            CliServerEvent::Usage {
                session_id: "root-session".to_string(),
                prompt_tokens: 12,
                completion_tokens: 34,
                message_id: Some("message-1".to_string()),
            },
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
        let run_tail = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .run_tail
            .clone();
        assert_eq!(
            run_tail.as_ref().map(|tail| tail.status.as_str()),
            Some("complete")
        );
        assert_eq!(
            run_tail.as_ref().and_then(|tail| tail.detail.as_deref()),
            Some("input 12 · output 34")
        );
    }

    #[test]
    fn session_busy_surfaces_compact_thinking_summary() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            CliServerEvent::SessionBusy {
                session_id: "root-session".to_string(),
            },
            &style,
        );

        let projection = runtime.frontend_projection.lock().expect("projection");
        assert_eq!(projection.active_label.as_deref(), Some("Thinking"));
    }

    #[test]
    fn tool_call_lifecycle_swaps_compact_summary_between_tool_and_thinking() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            CliServerEvent::SessionBusy {
                session_id: "root-session".to_string(),
            },
            &style,
        );
        handle_sse_event(
            &runtime,
            CliServerEvent::ToolCallStarted {
                session_id: "root-session".to_string(),
                tool_call_id: "tool-call-1".to_string(),
                tool_name: "SkillsList".to_string(),
            },
            &style,
        );
        {
            let projection = runtime.frontend_projection.lock().expect("projection");
            assert_eq!(
                projection.active_label.as_deref(),
                Some("Using Skill SkillsList")
            );
        }

        handle_sse_event(
            &runtime,
            CliServerEvent::ToolCallCompleted {
                session_id: "root-session".to_string(),
                tool_call_id: "tool-call-1".to_string(),
            },
            &style,
        );

        let projection = runtime.frontend_projection.lock().expect("projection");
        assert_eq!(projection.active_label.as_deref(), Some("Thinking"));
    }

    #[test]
    fn tool_call_start_updates_prompt_aux_lane_without_touching_root_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            CliServerEvent::ToolCallStarted {
                session_id: "root-session".to_string(),
                tool_call_id: "tool-call-1".to_string(),
                tool_name: "SkillsList".to_string(),
            },
            &style,
        );

        let transcript_rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(
            transcript_rendered, "● root line\n",
            "{transcript_rendered}"
        );
        let prompt_info = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .prompt_lanes
            .info_lines
            .clone();
        assert!(prompt_info
            .iter()
            .any(|line| line.contains("Using Skill SkillsList")));
    }

    #[test]
    fn interactive_tool_result_identity_enters_root_transcript_with_detail() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("tool-call-1"),
                Some(live_identity(
                    "assistant-1",
                    &rocode_types::tool_result_part_key("tool-call-1"),
                    LiveMessagePartKind::ToolResult,
                    LivePartPhase::End,
                    Some("tool-call-1"),
                )),
                serde_json::json!({
                    "kind": "tool",
                    "phase": "done",
                    "name": "SkillsList",
                    "detail": "{\"category\":\"literature-research/skills\"}"
                }),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(rendered.contains("Skill SkillsList"), "{rendered}");
        assert!(
            rendered.contains("literature-research/skills"),
            "{rendered}"
        );
    }

    #[test]
    fn runtime_aux_blocks_stay_in_prompt_lanes_when_history_regains_authority() {
        let runtime = test_runtime_with_attached_focus_data();

        cli_push_runtime_aux_block(
            &runtime,
            OutputBlock::Status(StatusBlock::title("Using Skill SkillsList")),
        );
        let prompt_info = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .prompt_lanes
            .info_lines
            .clone();
        assert!(prompt_info
            .iter()
            .any(|line| line.contains("Using Skill SkillsList")));

        let mut finalized = CliVisibleTranscript::default();
        finalized.append_rendered("● root line\n");
        finalized.append_rendered("● final answer\n");
        cli_replace_root_history_transcript(&runtime, finalized);
        let rebuilt = cli_sync_root_history_to_visible(&runtime);

        assert!(!rebuilt.rendered_text().contains("Using Skill SkillsList"));
        assert!(rebuilt.rendered_text().contains("final answer"));
    }

    #[test]
    fn session_retrying_stays_out_of_root_transcript_and_updates_run_tail() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            CliServerEvent::SessionRetrying {
                session_id: "root-session".to_string(),
            },
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
        let run_tail = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .run_tail
            .clone();
        assert_eq!(
            run_tail.as_ref().map(|tail| tail.status.as_str()),
            Some("retrying")
        );
        assert_eq!(
            run_tail.as_ref().and_then(|tail| tail.detail.as_deref()),
            Some("Waiting for automatic retry.")
        );
    }

    #[test]
    fn stream_reconnecting_surfaces_run_tail_and_stream_connected_clears_it() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            CliServerEvent::StreamReconnecting { delay_ms: 1_500 },
            &style,
        );

        let reconnecting = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .run_tail
            .clone();
        assert_eq!(
            reconnecting.as_ref().map(|tail| tail.status.as_str()),
            Some("reconnecting")
        );
        assert_eq!(
            reconnecting
                .as_ref()
                .and_then(|tail| tail.detail.as_deref()),
            Some("retrying in 2s")
        );

        handle_sse_event(&runtime, CliServerEvent::StreamConnected, &style);

        let run_tail = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .run_tail
            .clone();
        assert!(run_tail.is_none());
    }

    #[test]
    fn interactive_empty_assistant_boundaries_do_not_emit_blank_bullets() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some("assistant-1"),
                Some(live_identity(
                    "assistant-1",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Start,
                    Some("assistant-1"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "start",
                    "role": "assistant",
                    "text": ""
                }),
            ),
            &style,
        );
        handle_sse_event(
            &runtime,
            output_block_event(
                Some("assistant-1"),
                Some(live_identity(
                    "assistant-1",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::End,
                    Some("assistant-1"),
                )),
                serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
    }

    #[test]
    fn live_identity_render_does_not_fallback_to_raw_block_without_accumulator() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        let rendered = cli_render_session_block(
            &runtime,
            "root-session",
            None,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::full(
                "Thinking user".to_string(),
            )),
            Some(&live_identity(
                "assistant-1",
                rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
                LiveMessagePartKind::AssistantReasoning,
                LivePartPhase::Snapshot,
                Some("assistant-1"),
            )),
            &style,
        );

        assert!(rendered.is_empty(), "{rendered}");
    }

    #[test]
    fn live_reasoning_snapshots_reuse_visible_stream_header() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();
        let snapshot_identity = live_identity(
            "assistant-1",
            rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
            LiveMessagePartKind::AssistantReasoning,
            LivePartPhase::Snapshot,
            Some("assistant-1"),
        );
        let end_identity = live_identity(
            "assistant-1",
            rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
            LiveMessagePartKind::AssistantReasoning,
            LivePartPhase::End,
            Some("assistant-1"),
        );

        cli_observe_terminal_stream_block(
            &runtime,
            "root-session",
            Some("assistant-1"),
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::full(
                "Thinking first".to_string(),
            )),
        );
        assert_eq!(
            cli_render_session_block(
                &runtime,
                "root-session",
                Some("assistant-1"),
                &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::full(
                    "Thinking first".to_string(),
                )),
                Some(&snapshot_identity),
                &style,
            ),
            ""
        );

        cli_observe_terminal_stream_block(
            &runtime,
            "root-session",
            Some("assistant-1"),
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::full(
                "Thinking first second".to_string(),
            )),
        );
        let combined = cli_render_session_block(
            &runtime,
            "root-session",
            Some("assistant-1"),
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::end()),
            Some(&end_identity),
            &style,
        );
        assert_eq!(combined.matches("[thinking]").count(), 1, "{combined}");
        assert!(combined.contains("Thinking first second"), "{combined}");
    }

    #[test]
    fn non_transcript_live_identity_renders_raw_block_without_semantic_fallback() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        let rendered = cli_render_session_block(
            &runtime,
            "root-session",
            None,
            &OutputBlock::SchedulerStage(Box::new(stage_with_status("running"))),
            Some(&live_identity(
                "assistant-1",
                &rocode_types::scheduler_stage_part_key("stage-1"),
                LiveMessagePartKind::SchedulerStage,
                LivePartPhase::Snapshot,
                None,
            )),
            &style,
        );

        assert!(
            rendered.contains("Scheduler Stage") || rendered.contains("running"),
            "non-transcript live identity should render directly as raw block: {rendered}"
        );
        assert!(
            !rendered.contains("[message:assistant]"),
            "scheduler stage must not be reinterpreted as assistant transcript text: {rendered}"
        );
    }

    #[test]
    fn legacy_assistant_stream_renders_only_on_end() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();
        let start = OutputBlock::Message(MessageBlock::start(OutputMessageRole::Assistant));
        let delta = OutputBlock::Message(MessageBlock::delta(
            OutputMessageRole::Assistant,
            "Hello from buffered stream",
        ));
        let end = OutputBlock::Message(MessageBlock::end(OutputMessageRole::Assistant));

        cli_observe_terminal_stream_block(&runtime, "root-session", Some("assistant-1"), &start);
        let rendered_start = cli_render_session_block(
            &runtime,
            "root-session",
            Some("assistant-1"),
            &start,
            None,
            &style,
        );
        assert!(rendered_start.is_empty(), "{rendered_start}");

        cli_observe_terminal_stream_block(&runtime, "root-session", Some("assistant-1"), &delta);
        let rendered_delta = cli_render_session_block(
            &runtime,
            "root-session",
            Some("assistant-1"),
            &delta,
            None,
            &style,
        );
        assert!(rendered_delta.is_empty(), "{rendered_delta}");

        cli_observe_terminal_stream_block(&runtime, "root-session", Some("assistant-1"), &end);
        let rendered_end = cli_render_session_block(
            &runtime,
            "root-session",
            Some("assistant-1"),
            &end,
            None,
            &style,
        );
        assert!(
            rendered_end.contains("Hello from buffered stream"),
            "{rendered_end}"
        );
        assert!(
            rendered_end.matches("[message:assistant]").count() == 1,
            "expected exactly one assistant block header: {rendered_end}"
        );
    }

    #[test]
    fn live_tool_snapshot_replaces_same_slot_without_prefix_replay() {
        let style = CliStyle::plain();
        let identity = live_identity(
            "assistant-1",
            &rocode_types::tool_result_part_key("tool-1"),
            LiveMessagePartKind::ToolResult,
            LivePartPhase::Snapshot,
            Some("assistant-1"),
        );
        let first = cli_render_live_slot_snapshot(
            &OutputBlock::Tool(rocode_command::output_blocks::ToolBlock::done(
                "skill",
                Some("{\"category\":\"literature-research/skills\"}".to_string()),
            )),
            &identity,
            &style,
        );
        let second = cli_render_live_slot_snapshot(
            &OutputBlock::Tool(rocode_command::output_blocks::ToolBlock::done(
                "skill",
                Some("{\"category\":\"scientific-skills\"}".to_string()),
            )),
            &identity,
            &style,
        );

        let mut transcript = CliVisibleTranscript::default();
        let slot_key = rocode_types::live_slot_key(
            "assistant-1",
            &rocode_types::tool_result_part_key("tool-1"),
        );
        transcript.upsert_live_slot(&slot_key, first.clone(), first);
        transcript.upsert_live_slot(&slot_key, second.clone(), second);

        let rendered = transcript.rendered_text();
        assert!(
            rendered.contains("[tool:done] Skill :: {\"category\":\"scientific-skills\"}"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("literature-research/skills\"}{\"category\":\"scientific-skills"),
            "{rendered}"
        );
        assert_eq!(
            rendered.matches("[tool:done] Skill ::").count(),
            1,
            "{rendered}"
        );
    }

    #[test]
    fn live_reasoning_snapshot_in_rich_mode_stays_open_until_end() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let snapshot_identity = live_identity(
            "assistant-1",
            rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
            LiveMessagePartKind::AssistantReasoning,
            LivePartPhase::Snapshot,
            Some("assistant-1"),
        );
        let end_identity = live_identity(
            "assistant-1",
            rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY,
            LiveMessagePartKind::AssistantReasoning,
            LivePartPhase::End,
            Some("assistant-1"),
        );

        let mut transcript = CliVisibleTranscript::new(true);
        super::cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::full(
                "Thinking academic".to_string(),
            )),
            &snapshot_identity,
            &style,
        );

        let snapshot_rendered = strip_ansi(&transcript.rendered_text());
        assert!(
            snapshot_rendered.contains("THINKING"),
            "{snapshot_rendered}"
        );
        assert!(
            snapshot_rendered.contains("Thinking academic"),
            "{snapshot_rendered}"
        );
        assert!(
            !snapshot_rendered.contains(&"─".repeat(28)),
            "live snapshot must stay open without finalized divider: {snapshot_rendered}"
        );

        super::cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::end()),
            &end_identity,
            &style,
        );

        let committed_rendered = strip_ansi(&transcript.rendered_text());
        assert!(
            committed_rendered.contains(&"─".repeat(28)),
            "end phase must append the finalized divider exactly once: {committed_rendered}"
        );
        assert_eq!(
            committed_rendered.matches("THINKING").count(),
            1,
            "{committed_rendered}"
        );
    }

    #[test]
    fn shared_sample_preserves_five_assistant_messages_and_four_tool_cycles() {
        let runtime = test_runtime_with_attached_focus_data();
        let fixture = live_transcript_state_fixture();
        let style = CliStyle::plain();

        for entry in &fixture.shared_turn_cycles.entries {
            handle_sse_event(
                &runtime,
                output_block_event(
                    Some(&entry.message_id),
                    Some(entry.assistant_identity()),
                    serde_json::json!({
                        "kind": "message",
                        "phase": "full",
                        "role": "assistant",
                        "text": entry.message_text
                    }),
                ),
                &style,
            );
            if let Some(tool) = &entry.tool {
                handle_sse_event(
                    &runtime,
                    output_block_event(
                        Some(&tool.tool_id),
                        Some(tool.tool_result_identity(&entry.message_id)),
                        serde_json::json!({
                            "kind": "tool",
                            "phase": "done",
                            "name": tool.tool_name,
                            "detail": tool.tool_detail
                        }),
                    ),
                    &style,
                );
            }
        }

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();

        for entry in &fixture.shared_turn_cycles.entries {
            assert!(
                rendered.contains(&entry.message_text),
                "{rendered}"
            );
        }
        for entry in fixture
            .shared_turn_cycles
            .entries
            .iter()
            .filter_map(|entry| entry.tool.as_ref())
        {
            assert!(
                rendered.contains(&entry.tool_detail),
                "{rendered}"
            );
        }

        let assistant_count = fixture
            .shared_turn_cycles
            .entries
            .iter()
            .filter(|entry| rendered.contains(&entry.message_text))
            .count();
        let tool_count = fixture
            .shared_turn_cycles
            .entries
            .iter()
            .filter_map(|entry| entry.tool.as_ref())
            .filter(|tool| rendered.contains(&tool.tool_detail))
            .count();
        assert_eq!(
            assistant_count,
            fixture.shared_turn_cycles.expected.assistant_message_count,
            "{rendered}"
        );
        assert_eq!(
            tool_count,
            fixture.shared_turn_cycles.expected.tool_result_count,
            "{rendered}"
        );
    }

    #[test]
    fn shared_sample_tool_running_progress_stays_out_of_cli_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let fixture = live_transcript_state_fixture();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some(&fixture.tool_progress_exclusion.tool_running.tool_id),
                Some(fixture.tool_progress_exclusion.tool_running_identity()),
                serde_json::json!({
                    "kind": "tool",
                    "phase": "running",
                    "name": fixture.tool_progress_exclusion.tool_running.tool_name,
                    "detail": fixture.tool_progress_exclusion.tool_running.tool_detail
                }),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
    }

    #[test]
    fn shared_sample_scheduler_stage_identity_stays_out_of_cli_transcript() {
        let runtime = test_runtime_with_attached_focus_data();
        let fixture = live_transcript_state_fixture();
        let style = CliStyle::plain();

        handle_sse_event(
            &runtime,
            output_block_event(
                Some(&fixture.scheduler_stage_exclusion.stage_id),
                Some(fixture.scheduler_stage_exclusion.scheduler_identity()),
                fixture.scheduler_stage_exclusion.payload(),
            ),
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "● root line\n", "{rendered}");
    }

    #[test]
    fn shared_sample_run_tail_contract_matches_cli_status_surface_expectations() {
        let fixture = live_transcript_state_fixture();
        let run_tail = &fixture.run_tail_contract;

        assert_eq!(run_tail.completed_status, "complete");
        assert_eq!(run_tail.error_status, "error");
        assert_eq!(run_tail.awaiting_user_status, "awaiting_user");
        assert!(run_tail.completed_usage.input_tokens > 0);
        assert!(run_tail.completed_usage.output_tokens > 0);
        assert!(run_tail.completed_usage.reasoning_tokens > 0);
        assert!(run_tail.completed_usage.total_cost > 0.0);

        let usage_line = format!(
            "input {} · output {}",
            run_tail.completed_usage.input_tokens, run_tail.completed_usage.output_tokens
        );
        assert!(usage_line.contains("input"), "{usage_line}");
        assert!(usage_line.contains("output"), "{usage_line}");

        let error_line = format!("Run failed: {}", run_tail.error_message);
        assert!(error_line.contains("Run failed"), "{error_line}");
        assert!(error_line.contains(&run_tail.error_message), "{error_line}");
    }

    #[test]
    fn focus_attached_session_switches_visible_transcript_but_keeps_root_session() {
        let runtime = test_runtime_with_attached_focus_data();

        assert!(cli_focus_attached_session(&runtime, "attached-session-a")
            .expect("focus attached session"));

        let visible = runtime
            .frontend_projection
            .lock()
            .expect("frontend projection")
            .transcript
            .rendered_text();
        assert_eq!(visible, "● attached line\n");
        assert_eq!(runtime.server_session_id.as_deref(), Some("root-session"));
        assert_eq!(
            runtime
                .focused_session_id
                .lock()
                .expect("focused session")
                .as_deref(),
            Some("attached-session-a")
        );
        assert_eq!(
            runtime
                .frontend_projection
                .lock()
                .expect("frontend projection")
                .view_label
                .as_deref(),
            Some("view attached attached")
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
    fn cycle_attached_session_moves_forward_and_backward() {
        let runtime = test_runtime_with_multiple_attached_sessions();

        let first = cli_cycle_attached_session(&runtime, true)
            .expect("cycle next from root")
            .expect("first attached session");
        assert_eq!(first.0, "attached-session-a");
        assert_eq!((first.1, first.2), (1, 2));

        let second = cli_cycle_attached_session(&runtime, true)
            .expect("cycle next from first")
            .expect("second attached session");
        assert_eq!(second.0, "attached-session-b");
        assert_eq!((second.1, second.2), (2, 2));

        let previous = cli_cycle_attached_session(&runtime, false)
            .expect("cycle prev from second")
            .expect("previous attached session");
        assert_eq!(previous.0, "attached-session-a");
        assert_eq!((previous.1, previous.2), (1, 2));
    }

    #[test]
    fn focused_attached_session_appends_live_snapshots_into_terminal_history_model() {
        let runtime = test_runtime_with_attached_focus_data();
        let style = CliStyle::plain();

        cli_focus_attached_session(&runtime, "attached-session-a").expect("focus attached");

        handle_sse_event(
            &runtime,
            CliServerEvent::OutputBlock {
                session_id: "attached-session-a".to_string(),
                id: Some("assistant-attached".to_string()),
                live_identity: Some(live_identity(
                    "assistant-attached",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Snapshot,
                    Some("assistant-attached"),
                )),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "附属"
                }),
            },
            &style,
        );
        handle_sse_event(
            &runtime,
            CliServerEvent::OutputBlock {
                session_id: "attached-session-a".to_string(),
                id: Some("assistant-attached".to_string()),
                live_identity: Some(live_identity(
                    "assistant-attached",
                    rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY,
                    LiveMessagePartKind::AssistantText,
                    LivePartPhase::Snapshot,
                    Some("assistant-attached"),
                )),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "附属会话输出"
                }),
            },
            &style,
        );

        let attached_rendered = runtime
            .attached_session_transcripts
            .lock()
            .expect("attached transcripts")
            .get("attached-session-a")
            .expect("attached transcript")
            .rendered_text();
        assert_eq!(
            attached_rendered.matches("[message:assistant]").count(),
            1,
            "{attached_rendered}"
        );
        assert!(
            attached_rendered.contains("[message:assistant] 附属会话输出"),
            "{attached_rendered}"
        );

        let visible_rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        assert!(
            visible_rendered.contains("● attached line"),
            "{visible_rendered}"
        );
        assert!(
            !visible_rendered.contains("[message:assistant] 附属会话输出"),
            "{visible_rendered}"
        );
        assert!(
            !visible_rendered.contains("● root line"),
            "{visible_rendered}"
        );
    }

    #[test]
    fn cli_prompt_screen_lines_are_empty_for_transcript_first_mode() {
        let projection = CliFrontendProjection::default();
        assert!(cli_prompt_lane_screen_lines_from_projection(&projection).is_empty());
    }

    #[test]
    fn prompt_projection_screen_lines_use_tail_viewport_instead_of_full_transcript() {
        let mut projection = CliFrontendProjection::default();
        let mut transcript = CliVisibleTranscript::new(false);
        for index in 0..24 {
            transcript.append_rendered(&format!("line-{index}\n"));
        }
        projection.transcript = transcript;
        projection.active_label = Some("Thinking".to_string());

        let lines = cli_prompt_screen_lines_with_budget(&projection, 72, 5);
        let plain_lines = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert!(
            plain_lines.iter().any(|line| line.contains("Thinking")),
            "{plain_lines:?}"
        );
        assert!(
            !plain_lines.iter().any(|line| line == "line-0"),
            "{plain_lines:?}"
        );
        assert!(
            plain_lines.iter().any(|line| line == "line-23"),
            "{plain_lines:?}"
        );
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
            activity_started_at: None,
            view_label: Some("view attached attached-abc".to_string()),
            queue_len: 2,
            prompt_lanes: Default::default(),
            run_tail: None,
            active_stage: Some(stage_with_status("running")),
            session_runtime: None,
            stage_summaries: Vec::new(),
            telemetry_topology: None,
            events_browser: None,
            transcript: CliVisibleTranscript::default(),
            sidebar_collapsed: false,
            active_collapsed: false,
            session_title: Some("Test Session".to_string()),
            current_model_label: Some("openai/gpt-4.1".to_string()),
            scroll_offset: 0,
            token_stats: CliSessionTokenStats::default(),
            last_turn_tokens: CliLastTurnTokenStats::default(),
            cache_diagnostic: None,
            ingress_diagnostic: None,
            provider_diagnostic: None,
            pending_permission_count: 0,
            submitting_permission_count: 0,
            last_permission_submit_error: None,
            permission_submit_started_at: None,
            permission_submit_completed_at: None,
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
        assert!(joined.contains("view attached attached-abc"));
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
    fn footer_text_surfaces_attached_focus_state() {
        let projection = CliFrontendProjection {
            phase: CliFrontendPhase::Busy,
            view_label: Some("view attached abcd1234".to_string()),
            ..Default::default()
        };

        let footer = projection.footer_text();

        assert!(!footer.contains("Running"), "{footer}");
        assert!(footer.contains("view attached abcd1234"));
        assert!(footer.contains("/attached"));
        assert!(footer.contains("/abort"));
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
        rich_stage.attached_session_id = Some("attached-abc".to_string());

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
        assert!(joined_rich.contains("attached-abc"));
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

    #[test]
    fn compact_summary_clears_stale_non_error_aux_lines() {
        let mut projection = CliFrontendProjection::default();
        projection.phase = CliFrontendPhase::Busy;
        projection
            .prompt_lanes
            .push_aux_line(CliPromptAuxLane::Info, "Info: Using Skill SkillsList");
        projection.prompt_lanes.push_aux_line(
            CliPromptAuxLane::Warning,
            "Warning: Awaiting permission · Using Bash",
        );

        cli_restore_compact_summary(&mut projection);

        assert!(projection.prompt_lanes.info_lines.is_empty());
        assert!(projection.prompt_lanes.warning_lines.is_empty());
        assert_eq!(projection.active_label.as_deref(), Some("Thinking"));
    }

    #[test]
    fn prompt_aux_line_keeps_success_text_when_it_already_carries_the_label() {
        let line = cli_prompt_aux_line(&OutputBlock::Status(StatusBlock::success(
            "Done. tokens: prompt=1 completion=2",
        )))
        .expect("success status should map to prompt aux");

        assert_eq!(line.lane, CliPromptAuxLane::Info);
        assert_eq!(line.text, "Done. tokens: prompt=1 completion=2");
    }
}
