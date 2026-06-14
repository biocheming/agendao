//! 金 — Unified dialog layer authority.

pub mod backdrop;
pub mod dialog_stack;
pub mod alert;
pub mod help;
pub mod permission;
pub mod question;
pub mod agent_select;
pub mod confirm;
pub mod model_select;
pub mod prompt_stash;
pub mod provider;
pub mod session_export;
pub mod session_fork;
pub mod session_list;
pub mod session_rename;

pub use confirm::ConfirmDialog;
pub use dialog_stack::{DialogKind, DialogStack};
pub use alert::AlertDialog;
pub use help::HelpDialog;
pub use permission::{PermissionDialog, PermissionReply, PermissionRequest, PermissionType, PermissionLifetime};
pub use question::{QuestionDialog, QuestionRequest, QuestionOption};
pub use agent_select::{AgentSelectDialog, AgentEntry};
pub use prompt_stash::{StashDialog, StashEntry};
pub use model_select::{ModelSelectDialog, ModelEntry, ModelDialogOutcome, ProviderGroup};
pub use provider::{ProviderDialog, ProviderInfo as ProviderInfoDlg};
pub use session_export::{ExportAction, SessionExportDialog};
pub use session_fork::SessionForkDialog;
pub use session_list::{SessionListDialog, SessionEntry};
pub use session_rename::SessionRenameDialog;
