#[tokio::main]
async fn main() -> anyhow::Result<()> {
    agendao::init_logging();
    agendao_cli::spawn_process_reaper();
    agendao_cli::run_frontend().await
}
