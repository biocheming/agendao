use crate::runtime_events::ServerEvent;
use serde::{Deserialize, Serialize};

/// 携带单调递增序列号的包装事件
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequencedServerEvent {
    pub session_id: String,
    /// 单调递增事件序列号
    pub sequence: u64,
    /// 毫秒时间戳
    pub occurred_at_ms: i64,
    /// 具体的业务领域事件
    pub event: ServerEvent,
}

impl SequencedServerEvent {
    pub fn new(session_id: impl Into<String>, sequence: u64, event: ServerEvent) -> Self {
        Self {
            session_id: session_id.into(),
            sequence,
            occurred_at_ms: chrono::Utc::now().timestamp_millis(),
            event,
        }
    }
}
