//! 金 — Unified dialog layer authority.

pub mod dialog_stack;
pub mod alert;
pub mod help;
pub mod permission;
pub mod question;
pub mod agent_select;
pub mod model_select;

pub use dialog_stack::{DialogKind, DialogStack};
pub use alert::AlertDialog;
pub use help::HelpDialog;
pub use permission::{PermissionDialog, PermissionReply, PermissionRequest, PermissionType, PermissionLifetime};
pub use question::{QuestionDialog, QuestionRequest, QuestionOption};
pub use agent_select::{AgentSelectDialog, AgentEntry};
pub use model_select::{ModelSelectDialog, ModelEntry, ProviderGroup};
