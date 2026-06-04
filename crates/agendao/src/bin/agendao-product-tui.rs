fn main() -> anyhow::Result<()> {
    agendao::init_logging();
    agendao_tui::run_tui()
}
