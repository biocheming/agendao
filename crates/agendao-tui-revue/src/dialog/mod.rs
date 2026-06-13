//! 金 — Unified dialog layer authority.

pub mod dialog_stack;
pub mod alert;
pub mod help;
pub mod permission;
pub mod question;

pub use dialog_stack::{DialogKind, DialogStack};
pub use alert::AlertDialog;
pub use help::HelpDialog;
pub use permission::{PermissionDialog, PermissionReply, PermissionRequest, PermissionType, PermissionLifetime};
pub use question::{QuestionDialog, QuestionRequest, QuestionOption};
