use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockTone {
    Title,
    Normal,
    Muted,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBlock {
    pub tone: BlockTone,
    pub text: String,
}

impl StatusBlock {
    pub fn title(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Title,
            text: text.into(),
        }
    }

    pub fn normal(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Normal,
            text: text.into(),
        }
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Muted,
            text: text.into(),
        }
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Success,
            text: text.into(),
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Warning,
            text: text.into(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            tone: BlockTone::Error,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagePhase {
    Start,
    Delta,
    End,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningBlock {
    pub phase: MessagePhase,
    pub text: String,
}

impl ReasoningBlock {
    pub fn start() -> Self {
        Self {
            phase: MessagePhase::Start,
            text: String::new(),
        }
    }

    pub fn delta(text: impl Into<String>) -> Self {
        Self {
            phase: MessagePhase::Delta,
            text: text.into(),
        }
    }

    pub fn end() -> Self {
        Self {
            phase: MessagePhase::End,
            text: String::new(),
        }
    }

    pub fn full(text: impl Into<String>) -> Self {
        Self {
            phase: MessagePhase::Full,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageBlock {
    pub role: MessageRole,
    pub phase: MessagePhase,
    pub text: String,
}

impl MessageBlock {
    pub fn start(role: MessageRole) -> Self {
        Self {
            role,
            phase: MessagePhase::Start,
            text: String::new(),
        }
    }

    pub fn delta(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            phase: MessagePhase::Delta,
            text: text.into(),
        }
    }

    pub fn end(role: MessageRole) -> Self {
        Self {
            role,
            phase: MessagePhase::End,
            text: String::new(),
        }
    }

    pub fn full(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            phase: MessagePhase::Full,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPhase {
    Start,
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStructuredDetail {
    FileEdit {
        file_path: String,
        diff_preview: Option<String>,
    },
    FileWrite {
        file_path: String,
        bytes: Option<u64>,
        lines: Option<u64>,
        diff_preview: Option<String>,
    },
    FileRead {
        file_path: String,
        total_lines: Option<u64>,
        truncated: bool,
    },
    BashExec {
        command_preview: String,
        exit_code: Option<i64>,
        output_preview: Option<String>,
        truncated: bool,
    },
    Search {
        pattern: String,
        matches: Option<u64>,
        truncated: bool,
    },
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolBlock {
    pub name: String,
    pub phase: ToolPhase,
    pub detail: Option<String>,
    pub structured: Option<ToolStructuredDetail>,
}

impl ToolBlock {
    pub fn start(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: ToolPhase::Start,
            detail: None,
            structured: None,
        }
    }

    pub fn running(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: ToolPhase::Running,
            detail: Some(detail.into()),
            structured: None,
        }
    }

    pub fn done(name: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            name: name.into(),
            phase: ToolPhase::Done,
            detail,
            structured: None,
        }
    }

    pub fn error(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: ToolPhase::Error,
            detail: Some(detail.into()),
            structured: None,
        }
    }

    pub fn with_structured(mut self, detail: ToolStructuredDetail) -> Self {
        self.structured = Some(detail);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventField {
    pub label: String,
    pub value: String,
    pub tone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventBlock {
    pub event: String,
    pub title: String,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub fields: Vec<SessionEventField>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItemBlock {
    pub position: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectBlock {
    pub stage_ids: Vec<String>,
    pub events: Vec<InspectEventRow>,
    pub filter_stage_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectEventRow {
    pub ts: i64,
    pub event_type: String,
    pub execution_id: Option<String>,
    pub stage_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputBlock {
    Status(StatusBlock),
    Message(MessageBlock),
    Reasoning(ReasoningBlock),
    Tool(ToolBlock),
    SessionEvent(SessionEventBlock),
    QueueItem(QueueItemBlock),
    Inspect(InspectBlock),
}
