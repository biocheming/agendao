//! 金 — Unified dialog layer authority.

pub mod agent_select;
pub mod backdrop;
pub mod clipboard;
pub mod confirm;
pub mod help;
pub mod mcp_edit;
pub mod mcp_list;
pub mod mode_select;
pub mod model_edit;
pub mod model_select;
pub mod notifications;
pub mod permission;
pub mod plugin_edit;
pub mod prompt_stash;
pub mod provider_edit;
pub mod question;
pub mod recovery_list;
pub mod session_export;
pub mod session_fork;
pub mod session_list;
pub mod session_rename;
pub mod skill_list;
pub mod skill_proposal;
pub mod task_state;

pub use agent_select::{AgentEntry, AgentSelectDialog};
pub use confirm::ConfirmDialog;
pub use help::HelpDialog;
pub use mcp_edit::{McpEditAction, McpEditDialog, McpEditMode, McpEditSubmission, McpTransport};
pub use mcp_list::{McpAction, McpEntry, McpListDialog};
pub use mode_select::{ModeEntry, ModeSelectDialog};
pub use model_edit::{ModelEditAction, ModelEditDialog, ModelEditMode, ModelEditSubmission};
pub use model_select::{ModelDialogOutcome, ModelEntry, ModelSelectDialog, ProviderGroup};
pub use notifications::NotificationDialog;
pub use permission::{
    PermissionDialog, PermissionLifetime, PermissionReply, PermissionRequest, PermissionType,
};
pub use plugin_edit::{PluginEditAction, PluginEditDialog, PluginEditSubmission};
pub use prompt_stash::{StashDialog, StashEntry};
pub(crate) use provider_edit::ProviderEditField;
pub use provider_edit::{ProviderEditAction, ProviderEditDialog};
pub use question::{QuestionDialog, QuestionKeyOutcome, QuestionOption, QuestionRequest};
pub use recovery_list::{RecoveryAction, RecoveryEntry, RecoveryListDialog};
pub use session_export::{ExportAction, SessionExportDialog};
pub use session_fork::{ForkMessageOption, SessionForkDialog};
pub use session_list::{SessionEntry, SessionListAction, SessionListDialog};
pub use session_rename::SessionRenameDialog;
pub use skill_list::{SkillEntry, SkillListAction, SkillListDialog};
pub use skill_proposal::{SkillProposalAction, SkillProposalDialog, SkillProposalEntry};
pub use task_state::{TaskStateAction, TaskStateDialog};
