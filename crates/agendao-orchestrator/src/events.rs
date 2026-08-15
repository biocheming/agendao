use crate::context::Usage;
use crate::engine::Evaluation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionEvent {
    RunStarted,
    NodeStarted { path: String },
    NodeCompleted { path: String },
    LoopIteration { path: String, iteration: u32 },
    Evaluated { path: String, outcome: Evaluation },
    RunCompleted { usage: Usage },
    RunFailed { message: String },
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: ExecutionEvent);
}

pub(crate) struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: ExecutionEvent) {}
}

pub(crate) static NOOP_EVENT_SINK: NoopEventSink = NoopEventSink;
