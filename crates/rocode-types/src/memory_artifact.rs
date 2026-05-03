use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::MemoryRecord;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryArtifactVersion {
    #[serde(rename = "rocode-rust/memory/v1")]
    RocodeRustMemoryV1,
}

impl MemoryArtifactVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RocodeRustMemoryV1 => "rocode-rust/memory/v1",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryArtifactBundle {
    pub version: MemoryArtifactVersion,
    pub exported_at: i64,
    pub records: Vec<MemoryRecord>,
}

impl MemoryArtifactBundle {
    pub fn new(exported_at: i64, records: Vec<MemoryRecord>) -> Self {
        Self {
            version: MemoryArtifactVersion::RocodeRustMemoryV1,
            exported_at,
            records,
        }
    }

    pub fn new_now(records: Vec<MemoryRecord>) -> Self {
        Self::new(Utc::now().timestamp_millis(), records)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MemoryArtifactImportEnvelope {
    Bundle(MemoryArtifactBundle),
    Legacy(MemoryArtifactLegacyPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryArtifactLegacyPayload {
    /// Reserved explicit shape for future legacy adapters. Legacy payloads
    /// must self-identify instead of silently piggybacking on current bundle
    /// fields.
    pub legacy_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use crate::{
        MemoryArtifactBundle, MemoryArtifactImportEnvelope, MemoryArtifactVersion,
        MemoryEvidenceRef, MemoryKind, MemoryRecord, MemoryRecordId, MemoryScope, MemoryStatus,
        MemoryValidationStatus,
    };

    fn sample_record() -> MemoryRecord {
        MemoryRecord {
            id: MemoryRecordId("mem_1".to_string()),
            kind: MemoryKind::Preference,
            scope: MemoryScope::WorkspaceShared,
            status: MemoryStatus::Validated,
            title: "Prefer compact diffs".to_string(),
            summary: "Avoid sprawling output when a narrow patch is enough.".to_string(),
            trigger_conditions: vec!["editing".to_string()],
            normalized_facts: vec!["user prefers compact diffs".to_string()],
            boundaries: vec!["do not rewrite unrelated files".to_string()],
            confidence: Some(0.8),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some("session-1".to_string()),
                message_id: Some("message-1".to_string()),
                tool_call_id: None,
                stage_id: Some("stage-1".to_string()),
                note: Some("stated explicitly".to_string()),
            }],
            source_session_id: Some("session-1".to_string()),
            workspace_identity: Some("workspace-key".to_string()),
            created_at: 100,
            updated_at: 200,
            last_validated_at: Some(180),
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: Some("patch-review".to_string()),
            validation_status: MemoryValidationStatus::Passed,
        }
    }

    #[test]
    fn bundle_serializes_with_stable_version_and_records() {
        let bundle = MemoryArtifactBundle::new(123, vec![sample_record()]);

        let value = serde_json::to_value(&bundle).expect("bundle should serialize");

        assert_eq!(
            value["version"],
            serde_json::json!(MemoryArtifactVersion::RocodeRustMemoryV1.as_str())
        );
        assert_eq!(value["exported_at"], serde_json::json!(123));
        assert_eq!(value["records"].as_array().map(Vec::len), Some(1));
        assert!(value.get("contract").is_none());
        assert!(value.get("packet").is_none());
    }

    #[test]
    fn bundle_roundtrips_through_import_envelope() {
        let bundle = MemoryArtifactBundle::new(123, vec![sample_record()]);

        let payload = serde_json::to_string(&bundle).expect("bundle should serialize");
        let envelope: MemoryArtifactImportEnvelope =
            serde_json::from_str(&payload).expect("bundle should parse");

        match envelope {
            MemoryArtifactImportEnvelope::Bundle(parsed) => {
                assert_eq!(parsed.exported_at, 123);
                assert_eq!(parsed.records.len(), 1);
                assert_eq!(parsed.records[0].id.0, "mem_1");
                assert_eq!(parsed.records[0].status, MemoryStatus::Validated);
                assert_eq!(
                    parsed.records[0].validation_status,
                    MemoryValidationStatus::Passed
                );
            }
            MemoryArtifactImportEnvelope::Legacy(_) => panic!("expected bundle envelope"),
        }
    }

    #[test]
    fn import_envelope_rejects_unknown_bundle_version() {
        let payload = serde_json::json!({
            "version": "rocode-rust/memory/v999",
            "exported_at": 123,
            "records": [sample_record()]
        });

        let error = serde_json::from_value::<MemoryArtifactImportEnvelope>(payload)
            .expect_err("unknown version should fail closed");
        assert!(
            error.to_string().contains("did not match any variant")
                || error.to_string().contains("unknown variant")
        );
    }

    #[test]
    fn import_envelope_accepts_only_explicit_legacy_shape() {
        let payload = serde_json::json!({
            "legacy_format": "memory-alpha",
            "payload": {
                "records": [{"id": "legacy"}]
            }
        });

        let envelope: MemoryArtifactImportEnvelope =
            serde_json::from_value(payload).expect("explicit legacy shape should parse");

        match envelope {
            MemoryArtifactImportEnvelope::Legacy(legacy) => {
                assert_eq!(legacy.legacy_format, "memory-alpha");
                assert!(legacy.payload.is_some());
            }
            MemoryArtifactImportEnvelope::Bundle(_) => panic!("expected legacy envelope"),
        }
    }

    #[test]
    fn import_envelope_rejects_malformed_record_payload() {
        let payload = serde_json::json!({
            "version": "rocode-rust/memory/v1",
            "exported_at": 123,
            "records": [{
                "id": 42,
                "title": "broken"
            }]
        });

        let error = serde_json::from_value::<MemoryArtifactImportEnvelope>(payload)
            .expect_err("malformed record payload should fail");
        assert!(!error.to_string().is_empty());
    }
}
