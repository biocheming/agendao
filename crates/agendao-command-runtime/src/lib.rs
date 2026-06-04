#[cfg(feature = "runtime-hooks")]
pub mod cli_permission;
#[cfg(feature = "terminal-ui")]
pub use agendao_command::cli_panel;
#[cfg(feature = "terminal-ui")]
pub mod cli_prompt;
#[cfg(feature = "terminal-ui")]
pub mod cli_select;
#[cfg(feature = "terminal-ui")]
pub mod cli_spinner;
#[cfg(feature = "terminal-ui")]
pub use agendao_command::cli_style;
pub mod interactive;
pub use agendao_command::live_semantic_consumer;
pub use agendao_command::{
    ui_command_argument_kind, ResolvedUiCommand, UiActionId, UiCommandArgumentKind,
};
