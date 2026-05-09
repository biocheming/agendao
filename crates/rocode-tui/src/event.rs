use crossterm::event::{KeyEvent, MouseEvent};

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
    },
}
