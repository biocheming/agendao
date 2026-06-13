//! AgenDao TUI (Revue) — production binary entry point.
//!
//! Launches the Revue-based TUI frontend.
//! Transport: local-direct (default), unix socket (AGENDAO_UNIX_SOCKET), HTTP (AGENDAO_TUI_BASE_URL).
//! Built with: cargo build -p agendao --features product-tui-revue-bin --bin agendao-product-tui-revue

fn main() -> anyhow::Result<()> {
    agendao_tui_revue::config::init_logging();
    agendao_tui_revue::run_app()
}
