use crate::cli::{InteractiveCliMode, RunCommandArgs};
use crate::run::{run_non_interactive, RunNonInteractiveOptions};
use crate::server_lifecycle::FrontendRuntimeContext;

pub(crate) async fn dispatch_interactive_command(
    args: RunCommandArgs,
    forced_interactive_mode: Option<InteractiveCliMode>,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    if matches!(forced_interactive_mode, Some(InteractiveCliMode::Rich))
        && args.interactive_mode != InteractiveCliMode::Rich
    {
        anyhow::bail!(
            "agendao cli always uses rich interactive mode; use `agendao run --interactive-mode ...` for other modes"
        );
    }

    run_non_interactive(
        run_options_from_args(args, forced_interactive_mode)?,
        runtime_context,
    )
    .await
}

fn run_options_from_args(
    args: RunCommandArgs,
    interactive_mode: Option<InteractiveCliMode>,
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
        interactive_mode: interactive_mode.unwrap_or(args.interactive_mode),
        unix_socket: agendao_cli_core::resolve_socket_path(args.socket)?,
    })
}
