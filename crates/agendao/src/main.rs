mod host;
mod product_cli;

fn init_logging() {
    let log_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("agendao")
        .join("log");
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("agendao.log"))
        .ok();
    if let Some(file) = log_file {
        use tracing_subscriber::EnvFilter;
        let default_level = if cfg!(debug_assertions) {
            "debug"
        } else {
            "warn"
        };
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .init();
    } else {
        use tracing_subscriber::EnvFilter;
        let default_level = if cfg!(debug_assertions) {
            "debug"
        } else {
            "warn"
        };
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level)),
            )
            .init();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();
    agendao_cli::spawn_process_reaper();
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if product_cli::dispatch_if_product_command(args.clone()).await? {
        return Ok(());
    }
    agendao_cli::run_frontend_with_context(args, host::frontend_runtime_context()).await
}
