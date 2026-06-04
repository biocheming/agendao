use crate::cli::{Cli, Commands};
use crate::frontend_admin::dispatch_admin_command;
use crate::frontend_interactive::dispatch_interactive_command;
use crate::server_lifecycle::FrontendRuntimeContext;

pub(crate) async fn dispatch_cli_command(
    cli: Cli,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    match cli.command {
        Commands::Run { args } => dispatch_interactive_command(args, None, runtime_context).await?,
        Commands::Cli { args } => {
            dispatch_interactive_command(
                args.run,
                Some(crate::cli::InteractiveCliMode::Rich),
                runtime_context,
            )
            .await?;
        }
        command => {
            dispatch_admin_command(command, runtime_context).await?;
        }
    }

    Ok(())
}
