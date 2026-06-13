use crossterm::event::{KeyEvent, MouseEvent};
use agendao_command::UiActionId;
use agendao_server_core::frontend_events::FrontendEvent;
use crate::components::Prompt;
#[derive(Clone, Debug)]
pub enum PermissionReplyOutcome {
    Succeeded,
    Failed { message: String },
}

#[derive(Clone, Debug)]
pub enum SessionDeleteOutcome {
    Succeeded,
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionNavigationIntentKind {
    Parent,
    Attached,
    Session(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionSidebarIntentKind {
    KillSelectedProcess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashPopupIntentKind {
    Close,
    MoveUp,
    MoveDown,
    SelectCurrent,
}

#[derive(Clone, Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    FocusGained,
    FocusLost,
    Paste(String),
    Custom(Box<CustomEvent>),
}

#[derive(Clone)]
pub enum CustomEvent {
    Message(String),
    StreamChunk(String),
    StreamComplete,
    StreamError(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallComplete {
        id: String,
        result: String,
    },
    PromptDispatchHomeFinished {
        optimistic_session_id: String,
        optimistic_message_id: String,
        created_session: Option<Box<crate::api::SessionInfo>>,
        response: Option<crate::api::PromptResponse>,
        error: Option<String>,
    },
    PromptDispatchSessionFinished {
        session_id: String,
        optimistic_message_id: String,
        response: Option<crate::api::PromptResponse>,
        error: Option<String>,
    },
    PermissionReplyFinished {
        permission_id: String,
        outcome: PermissionReplyOutcome,
    },
    SessionDeleteFinished {
        session_id: String,
        outcome: SessionDeleteOutcome,
    },
    SessionTelemetryRefreshFinished {
        session_id: String,
        telemetry: Option<Box<crate::api::SessionTelemetrySnapshot>>,
    },
    SessionNavigationIntent {
        kind: SessionNavigationIntentKind,
    },
    SessionSidebarIntent {
        kind: SessionSidebarIntentKind,
    },
    SlashPopupIntent {
        kind: SlashPopupIntentKind,
    },
    SessionInterruptRequested,
    PromptEdited {
        prompt: Box<Prompt>,
    },
    PromptSubmitRequested {
        prompt: Box<Prompt>,
    },
    PromptPasteText {
        text: String,
    },
    UiActionRequested {
        action: UiActionId,
    },
    SessionUpdated {
        session_id: String,
        source: Option<String>,
    },
    SessionStatusReconnecting {
        session_id: String,
    },
    ShellDispatchFinished {
        /// The optimistic session id that was created on the Home path.
        /// Used so the handler can promote it once the real session is ready.
        optimistic_session_id: String,
        optimistic_message_id: String,
        /// The real session id returned by the server (Home path) or the
        /// existing session id (Session path).
        session_id: String,
        /// The full session info from `create_session` (Home path only).
        /// Passed so the handler can call `promote_optimistic_session`.
        created_session: Option<Box<crate::api::SessionInfo>>,
        /// `true` when the user explicitly cancelled the in-flight dispatch.
        cancelled: bool,
        error: Option<String>,
    },
    FrontendEvent(Box<FrontendEvent>),
}

impl std::fmt::Debug for CustomEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(value) => f.debug_tuple("Message").field(value).finish(),
            Self::StreamChunk(value) => f.debug_tuple("StreamChunk").field(value).finish(),
            Self::StreamComplete => f.write_str("StreamComplete"),
            Self::StreamError(value) => f.debug_tuple("StreamError").field(value).finish(),
            Self::ToolCallStart { id, name } => f
                .debug_struct("ToolCallStart")
                .field("id", id)
                .field("name", name)
                .finish(),
            Self::ToolCallComplete { id, result } => f
                .debug_struct("ToolCallComplete")
                .field("id", id)
                .field("result", result)
                .finish(),
            Self::PromptDispatchHomeFinished {
                optimistic_session_id,
                optimistic_message_id,
                created_session,
                response,
                error,
            } => f
                .debug_struct("PromptDispatchHomeFinished")
                .field("optimistic_session_id", optimistic_session_id)
                .field("optimistic_message_id", optimistic_message_id)
                .field("created_session", created_session)
                .field("response", response)
                .field("error", error)
                .finish(),
            Self::PromptDispatchSessionFinished {
                session_id,
                optimistic_message_id,
                response,
                error,
            } => f
                .debug_struct("PromptDispatchSessionFinished")
                .field("session_id", session_id)
                .field("optimistic_message_id", optimistic_message_id)
                .field("response", response)
                .field("error", error)
                .finish(),
            Self::PermissionReplyFinished {
                permission_id,
                outcome,
            } => f
                .debug_struct("PermissionReplyFinished")
                .field("permission_id", permission_id)
                .field("outcome", outcome)
                .finish(),
            Self::SessionDeleteFinished { session_id, outcome } => f
                .debug_struct("SessionDeleteFinished")
                .field("session_id", session_id)
                .field("outcome", outcome)
                .finish(),
            Self::SessionTelemetryRefreshFinished { session_id, telemetry } => f
                .debug_struct("SessionTelemetryRefreshFinished")
                .field("session_id", session_id)
                .field("telemetry", telemetry)
                .finish(),
            Self::SessionNavigationIntent { kind } => f
                .debug_struct("SessionNavigationIntent")
                .field("kind", kind)
                .finish(),
            Self::SessionSidebarIntent { kind } => f
                .debug_struct("SessionSidebarIntent")
                .field("kind", kind)
                .finish(),
            Self::SlashPopupIntent { kind } => f
                .debug_struct("SlashPopupIntent")
                .field("kind", kind)
                .finish(),
            Self::SessionInterruptRequested => {
                f.write_str("SessionInterruptRequested")
            }
            Self::PromptEdited { .. } => f.write_str("PromptEdited(..)"),
            Self::PromptSubmitRequested { .. } => f.write_str("PromptSubmitRequested(..)"),
            Self::PromptPasteText { text } => f
                .debug_struct("PromptPasteText")
                .field("text", text)
                .finish(),
            Self::UiActionRequested { action } => f
                .debug_struct("UiActionRequested")
                .field("action", action)
                .finish(),
            Self::SessionUpdated { session_id, source } => f
                .debug_struct("SessionUpdated")
                .field("session_id", session_id)
                .field("source", source)
                .finish(),
            Self::SessionStatusReconnecting { session_id } => f
                .debug_struct("SessionStatusReconnecting")
                .field("session_id", session_id)
                .finish(),
            Self::ShellDispatchFinished {
                optimistic_session_id,
                optimistic_message_id,
                session_id,
                created_session,
                cancelled,
                error,
            } => f
                .debug_struct("ShellDispatchFinished")
                .field("optimistic_session_id", optimistic_session_id)
                .field("optimistic_message_id", optimistic_message_id)
                .field("session_id", session_id)
                .field("created_session", created_session)
                .field("cancelled", cancelled)
                .field("error", error)
                .finish(),
            Self::FrontendEvent(event) => {
                f.debug_tuple("FrontendEvent").field(event).finish()
            }
        }
    }
}
