//! AgenDao TUI (Revue) — standalone binary entry point.

fn main() -> anyhow::Result<()> {
    agendao_tui_revue::config::init_logging();
    agendao_tui_revue::run_app()
}
