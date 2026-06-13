fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    agendao_tui_revue::run_app()
}
