mod local_dispatch;
mod local_server_bridge;
mod model_state;
mod session_exec;

use std::path::PathBuf;
use std::sync::Arc;

use crate::api_client::CliApiClient;
use crate::cli::RunCommandArgs;
use crate::cli::RunOutputFormat;
use crate::remote::{run_non_interactive_attach, RemoteAttachOptions};
use crate::server_lifecycle::CliRuntimeContext;
use crate::util::{append_cli_file_attachments, collect_run_input};
use model_state::{cli_resolve_show_thinking, cli_save_recent_model_ref};
use session_exec::{cli_session_directory, run_cli_prompt_local, run_cli_prompt_transport};

pub(super) async fn run_non_interactive(
    options: RunNonInteractiveOptions,
    runtime_context: &CliRuntimeContext,
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
        unix_socket,
    } = options;
    let use_http = attach.is_some();
    let use_socket = !use_http && unix_socket.is_some();
    let direct_requested = !use_http && !use_socket;
    let use_direct = direct_requested && local_server_bridge::direct_mode_available();
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
        anyhow::bail!(
            "`agendao run` no longer starts an interactive session. Use `agendao tui` for interactive use, or pass a MESSAGE for non-interactive execution."
        );
    }

    let base_url = if use_direct {
        "http://127.0.0.1:0".to_string()
    } else if let Some(base_url) = attach {
        base_url
    } else {
        runtime_context
            .discover_or_start_server_with_request(crate::ServerDiscoveryRequest {
                port_override: port,
                cwd: Some(working_dir.clone()),
                unix_socket_path: unix_socket.clone(),
            })
            .await?
    };
    let api_client = CliApiClient::new(base_url.clone());
    let local_server: Option<Arc<local_server_bridge::CliLocalServerState>> = if use_direct {
        eprintln!("Starting CLI in Direct (in-process) mode");
        Some(
            local_server_bridge::create_local_server_state(
                "http://127.0.0.1:0".to_string(),
                working_dir.clone(),
            )
            .await?,
        )
    } else {
        None
    };
    let transport = if use_direct {
        None
    } else if let Some(socket_path) = unix_socket.as_deref() {
        agendao_client::transport::TransportSelector::new(
            Some(socket_path.to_string()),
            base_url.clone(),
            None,
        )
        .select_unix_required()
        .await
        .map(Arc::new)
        .map(Some)?
    } else {
        None
    };
    let remote_context =
        local_dispatch::get_workspace_context(&local_server, &transport, &api_client)
            .await
            .ok();
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
        cli_save_recent_model_ref(&local_server, &transport, &api_client, model_ref).await;
    }

    if let Some(local) = local_server {
        run_cli_prompt_local(
            &local,
            &input,
            command.as_deref(),
            continue_last,
            session.as_deref(),
            fork,
            model.as_deref(),
            requested_agent.as_deref(),
            variant.as_deref(),
            title.as_deref(),
            &cli_session_directory(&working_dir),
        )
        .await?;
        return Ok(());
    }

    if let Some(socket_path) = unix_socket.as_deref() {
        if let Some(transport) = transport.as_deref() {
            if let agendao_client::FrontendTransport::Unix(_) = transport {
                eprintln!("Connected via Unix socket: {}", socket_path);
            }
            run_cli_prompt_transport(
                transport,
                &input,
                command.as_deref(),
                model.as_deref(),
                requested_agent.as_deref(),
                variant.as_deref(),
            )
            .await?;
            return Ok(());
        }
        anyhow::bail!("Unix socket mode requested but no Unix transport is available");
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

pub(super) fn run_options_from_args(
    args: RunCommandArgs,
) -> anyhow::Result<RunNonInteractiveOptions> {
    Ok(RunNonInteractiveOptions {
        message: args.message,
        command: args.command,
        continue_last: args.continue_last,
        session: args.session,
        fork: args.fork,
        share: args.share,
        model: args.model,
        requested_agent: args.agent,
        requested_scheduler_profile: args.scheduler_profile,
        files: args.file,
        format: args.format,
        title: args.title,
        attach: args.attach,
        dir: args.dir,
        port: args.port,
        variant: args.variant,
        thinking: args.thinking,
        unix_socket: agendao_cli_core::resolve_socket_path(args.socket)?,
    })
}

pub(super) struct RunNonInteractiveOptions {
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
    pub unix_socket: Option<String>,
}
