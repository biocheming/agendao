use agendao_types::{ControlInputKind, ControlInputPhase};
use crossterm::event::{KeyEvent, MouseEvent};

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

#[derive(Clone, Debug)]
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
    StateChanged(StateChange),
}

#[derive(Clone, Debug)]
pub enum StateChange {
    SessionCreated(String),
    SessionUpdated {
        session_id: String,
        source: Option<String>,
    },
    SessionStatusBusy(String),
    SessionStatusCompacting(String),
    SessionStatusIdle(String),
    SessionStatusReconnecting(String),
    SessionStatusRetrying {
        session_id: String,
        attempt: u32,
        message: String,
        next: i64,
    },
    SessionDeleted(String),
    ModelChanged(String),
    AgentChanged(String),
    ProviderConnected(String),
    ProviderDisconnected(String),
    ConfigUpdated,
    McpServerStatusChanged {
        name: String,
        status: String,
    },
    TodoUpdated,
    DiffUpdated {
        session_id: String,
        diffs: Vec<crate::context::DiffEntry>,
    },
    ProcessesUpdated,
    QuestionCreated {
        session_id: String,
        request_id: String,
    },
    QuestionResolved {
        session_id: String,
        request_id: String,
    },
    PermissionRequested {
        session_id: String,
        permission: crate::api::PermissionRequestInfo,
    },
    PermissionResolved {
        session_id: String,
        permission_id: String,
    },
    ControlInputTransition {
        session_id: String,
        kind: ControlInputKind,
        phase: ControlInputPhase,
        at: i64,
    },
    ToolCallStarted {
        session_id: String,
        tool_call_id: String,
        tool_name: String,
    },
    ToolCallCompleted {
        session_id: String,
        tool_call_id: String,
    },
    TopologyChanged {
        session_id: String,
    },
    OutputBlock {
        session_id: String,
        id: Option<String>,
        payload: serde_json::Value,
        live_identity: Option<agendao_types::LiveMessagePartIdentity>,
    },
}
