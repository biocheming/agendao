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
pub use keybind::{
    is_primary_key_event, normalize_key_event, Keybind, KeybindRegistry, LeaderKeyState,
};
pub use session_context::{
    collect_attached_sessions, collect_attached_sessions_from_stage_summaries, AttachedSessionInfo,
    DiffEntry, Message, MessagePart, MessageRole, RevertInfo, Session, SessionContext,
    SessionStatus, TodoItem, TodoStatus, TokenUsage,
};
