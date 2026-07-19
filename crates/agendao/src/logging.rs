pub fn init_logging() {
    // 日志统一收在 agendao_home/log（~/.agendao，土律·单点权威）。
    let log_dir = agendao_util::agendao_home().join("log");
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("agendao.log"))
        .ok();
    if let Some(file) = log_file {
        use tracing_subscriber::EnvFilter;
        let default_level = "warn";
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .init();
    } else {
        use tracing_subscriber::EnvFilter;
        let default_level = "warn";
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level)),
            )
            .init();
    }
}
