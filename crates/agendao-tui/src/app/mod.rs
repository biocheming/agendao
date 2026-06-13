#[path = "app.rs"]
mod app_impl;
mod state;
pub(crate) mod terminal;

pub use app_impl::{App, AppLaunchConfig, RunOutcome};
pub use state::AppState;

pub(crate) use app_impl::{BridgeIterationOutcome, BridgeWaitStrategy, ReactiveDialogLayerSnapshot};
