//! AgenDao TUI (Revue) — production binary entry point.
//!
//! Launches the Revue-based TUI frontend.
//! Built with: cargo build -p agendao --features product-tui-revue-bin --bin agendao-product-tui-revue

fn main() -> anyhow::Result<()> {
    agendao::init_logging();
    agendao_tui_revue::run_app()
}
