//! 金 — Unified dialog layer authority.
//!
//! DialogStack manages all modal dialogs through a Signal-based stack.
//! Each dialog kind has its own state, key handling, and rendering.

pub mod dialog_stack;
pub mod alert;
pub mod help;

pub use dialog_stack::{DialogKind, DialogStack};
pub use alert::AlertDialog;
pub use help::HelpDialog;
