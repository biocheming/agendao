mod host;
mod product_cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    agendao::init_logging();
    agendao_cli::spawn_process_reaper();
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if product_cli::dispatch_if_product_command(args.clone()).await? {
        return Ok(());
    }
    agendao_cli::run_cli_with_context(args, host::cli_runtime_context()).await
}
