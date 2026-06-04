pub mod api;
pub mod app;
pub mod branding;
pub mod bridge;
pub mod client;
pub mod command;
pub mod components;
pub mod context;
pub mod core;
pub mod event;
pub mod file_index;
pub mod hooks;
pub mod local_server_bridge;
pub mod render;
pub mod router;
pub mod state;
pub mod terminal;
pub mod theme;
pub mod ui;

pub use api::ApiClient;
pub use app::{App, AppLaunchConfig, RunOutcome};
pub use core::{AppContext, Event, Route, Router, Theme};
pub use terminal::{reset_title, set_session_title, set_title};

fn setup_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        eprintln!("\n\nPANIC: {}", panic_info);
    }));
}

pub fn run_tui() -> anyhow::Result<()> {
    run_tui_with_config(AppLaunchConfig::default())
}

pub fn run_tui_with_config(config: AppLaunchConfig) -> anyhow::Result<()> {
    setup_panic_hook();

    let app = App::new_with_config(config)?;
    let result = app.run();
    if let Ok(outcome) = &result {
        if let Some(summary) = outcome.exit_summary.as_deref() {
            println!("{summary}");
        }
    }
    result.map(|_| ())
}
