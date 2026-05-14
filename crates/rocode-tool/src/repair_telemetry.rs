//! Repair telemetry — the single authority for recording tool-call repairs.
//!
//! ## Three-Layer Argument Contract (P1.1)
//!
//! Every tool-call repair event distinguishes three layers of arguments:
//!
//! | Layer | Name | Source | Stored In | Used For |
//! |-------|------|--------|-----------|----------|
//! | 1 | **Raw** | Model output (unmodified) | `PartType::ToolCall.raw`, `RepairEvent.raw_shape` | API replay, cache stability, model evaluation |
//! | 2 | **Normalized** | After system repair/correction | `RepairEvent.normalized_shape`, execution `effective_input` | Tool execution, history replay |
//! | 3 | **Observable** | Derived from normalized + repair events | UI/transcript/debug views | Human readability, debugging |
//!
//! Rules:
//! - Raw args MUST be preserved byte-for-byte for replay fidelity.
//! - Normalized args are what the tool actually executes with.
//! - Observable args are for display only; they MUST NOT be used for replay.
//! - `RepairEvent.raw_shape` records layer 1, `normalized_shape` records layer 2.
//! - When a repair does not change the shape, both fields may be absent.

use crate::Metadata;
use rocode_types::{RepairEvent, RepairEventBuilder};
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

// ── Legacy loose-map API (backward-compatible) ──────────────────────────

/// Create a loose repair event map. Prefer `structured_repair_event` for new code.
pub fn tool_repair_event(kind: &str, layer: &str, tool: &str) -> Map<String, Value> {
    let event = RepairEvent::new(kind, layer, tool);
    event.to_loose_map()
}

/// Create a structured `RepairEvent`. This is the preferred API for new code.
pub fn structured_repair_event(
    kind: impl Into<String>,
    layer: impl Into<String>,
    tool: impl Into<String>,
) -> RepairEvent {
    RepairEvent::new(kind, layer, tool)
}

/// Create a `RepairEventBuilder` for fluent construction with optional fields.
pub fn repair_event_builder(
    kind: impl Into<String>,
    layer: impl Into<String>,
    tool: impl Into<String>,
) -> RepairEventBuilder {
    RepairEventBuilder::new(kind, layer, tool)
}

// ── Append helpers ──────────────────────────────────────────────────────

/// Append a loose event map to metadata. Still works; delegates to
/// the structured path internally.
pub fn append_tool_repair_event_map(metadata: &mut Metadata, event: Map<String, Value>) {
    append_tool_repair_event(metadata, Value::Object(event));
}

/// Append a loose Value event to metadata.
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

/// Append a structured `RepairEvent` to metadata.
/// Converts to the loose format for storage compatibility, then delegates.
pub fn append_structured_repair_event(metadata: &mut Metadata, event: &RepairEvent) {
    append_tool_repair_event_map(metadata, event.to_loose_map());
}

// ── Read helpers ────────────────────────────────────────────────────────

/// Read repair events as loose `Value` objects (backward-compatible).
pub fn tool_repair_events(metadata: &Metadata) -> Vec<Value> {
    metadata
        .get(TOOL_REPAIR_TELEMETRY_KEY)
        .and_then(|value| value.get("events"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Read repair events as structured `RepairEvent` values.
/// Falls back gracefully on malformed events.
pub fn structured_repair_events(metadata: &Metadata) -> Vec<RepairEvent> {
    tool_repair_events(metadata)
        .into_iter()
        .filter_map(|value| value.as_object().and_then(RepairEvent::from_loose_map))
        .collect()
}

// ── Merge ───────────────────────────────────────────────────────────────

/// Merge repair telemetry from source into target metadata.
pub fn merge_tool_repair_telemetry(target: &mut Metadata, source: &Metadata) {
    for event in tool_repair_events(source) {
        append_tool_repair_event(target, event);
    }
}

/// Merge structured repair events directly.
pub fn merge_structured_repair_telemetry(target: &mut Metadata, events: &[RepairEvent]) {
    for event in events {
        append_structured_repair_event(target, event);
    }
}

// ── Convenience: one-shot record ────────────────────────────────────────

/// Record a single structured repair event in one call.
/// This is the recommended pattern for new tool implementations.
pub fn record_repair_event(
    metadata: &mut Metadata,
    kind: impl Into<String>,
    layer: impl Into<String>,
    tool: impl Into<String>,
    extra: impl FnOnce(&mut RepairEventBuilder) -> &mut RepairEventBuilder,
) {
    let mut builder = repair_event_builder(kind, layer, tool);
    extra(&mut builder);
    append_structured_repair_event(metadata, &builder.build());
}
