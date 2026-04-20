mod app_context;
pub mod keybind;
mod session_context;

pub use app_context::{
    AppContext, DialogLifecycleState, DialogSlot, LspConnectionStatus, LspStatus,
    McpConnectionStatus, McpServerStatus, MessageDensity, ModelInfo, ProviderInfo, SelectionState,
    SidebarLifecycleState, SidebarMode, SidebarTab, StatusDialogView, TuiEventsBrowserState,
    TuiMemoryConsolidationState, TuiMemoryDetailState, TuiMemoryListState, TuiMemoryPreviewState,
    TuiMemoryRuleHitsState, UiPreferencesState, SESSION_SIDEBAR_WIDE_THRESHOLD,
};
pub use keybind::{Keybind, KeybindRegistry};
pub use session_context::{
    collect_child_sessions, ChildSessionInfo, DiffEntry, Message, MessagePart, MessageRole,
    RevertInfo, Session, SessionContext, SessionStatus, TodoItem, TodoStatus, TokenUsage,
};
