use crate::Metadata;
use serde_json::{Map, Value};

pub const TOOL_REPAIR_TELEMETRY_KEY: &str = "toolRepairTelemetry";
const TOOL_REPAIR_TELEMETRY_VERSION: u64 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolArgumentNormalizationTelemetry {
    pub modes: Vec<String>,
}

impl ToolArgumentNormalizationTelemetry {
    pub fn record(&mut self, mode: &str) {
        if !self.modes.iter().any(|existing| existing == mode) {
            self.modes.push(mode.to_string());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.modes.is_empty()
    }
}

pub fn tool_repair_event(kind: &str, layer: &str, tool: &str) -> Map<String, Value> {
    let mut event = Map::new();
    event.insert("kind".to_string(), Value::String(kind.to_string()));
    event.insert("layer".to_string(), Value::String(layer.to_string()));
    event.insert("tool".to_string(), Value::String(tool.to_string()));
    event
}

pub fn append_tool_repair_event_map(metadata: &mut Metadata, event: Map<String, Value>) {
    append_tool_repair_event(metadata, Value::Object(event));
}

pub fn append_tool_repair_event(metadata: &mut Metadata, event: Value) {
    if !event.is_object() {
        return;
    }

    let telemetry = metadata
        .entry(TOOL_REPAIR_TELEMETRY_KEY.to_string())
        .or_insert_with(|| {
            serde_json::json!({
                "version": TOOL_REPAIR_TELEMETRY_VERSION,
                "events": [],
            })
        });

    if let Some(obj) = telemetry.as_object_mut() {
        obj.entry("version".to_string())
            .or_insert_with(|| Value::from(TOOL_REPAIR_TELEMETRY_VERSION));
        let events = obj
            .entry("events".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(items) = events.as_array_mut() {
            items.push(event);
        }
    }
}

pub fn tool_repair_events(metadata: &Metadata) -> Vec<Value> {
    metadata
        .get(TOOL_REPAIR_TELEMETRY_KEY)
        .and_then(|value| value.get("events"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub fn merge_tool_repair_telemetry(target: &mut Metadata, source: &Metadata) {
    for event in tool_repair_events(source) {
        append_tool_repair_event(target, event);
    }
}
