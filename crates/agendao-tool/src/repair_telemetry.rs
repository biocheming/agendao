//! Typed repair telemetry storage for tool-call normalization and recovery.

use crate::Metadata;
pub use agendao_types::{RepairEvent, RepairEventBuilder};
use serde::{Deserialize, Serialize};

pub const TOOL_REPAIR_TELEMETRY_KEY: &str = "toolRepairTelemetry";
const TOOL_REPAIR_TELEMETRY_VERSION: u64 = 1;
const MAX_TOOL_REPAIR_EVENTS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepairTelemetryEnvelope {
    version: u64,
    events: Vec<RepairEvent>,
}

impl Default for RepairTelemetryEnvelope {
    fn default() -> Self {
        Self {
            version: TOOL_REPAIR_TELEMETRY_VERSION,
            events: Vec::new(),
        }
    }
}

pub fn repair_event_builder(
    kind: impl Into<String>,
    layer: impl Into<String>,
    tool: impl Into<String>,
) -> RepairEventBuilder {
    RepairEventBuilder::new(kind, layer, tool)
}

pub fn append_repair_event(metadata: &mut Metadata, event: RepairEvent) {
    let mut envelope: RepairTelemetryEnvelope = metadata
        .get(TOOL_REPAIR_TELEMETRY_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    envelope.events.push(event);
    let overflow = envelope.events.len().saturating_sub(MAX_TOOL_REPAIR_EVENTS);
    if overflow > 0 {
        envelope.events.drain(..overflow);
    }
    metadata.insert(
        TOOL_REPAIR_TELEMETRY_KEY.to_string(),
        serde_json::to_value(envelope).expect("repair telemetry envelope is serializable"),
    );
}

pub fn repair_events(metadata: &Metadata) -> Vec<RepairEvent> {
    metadata
        .get(TOOL_REPAIR_TELEMETRY_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<RepairTelemetryEnvelope>(value).ok())
        .map(|envelope| envelope.events)
        .unwrap_or_default()
}

pub fn merge_repair_telemetry(target: &mut Metadata, source: &Metadata) {
    for event in repair_events(source) {
        append_repair_event(target, event);
    }
}
