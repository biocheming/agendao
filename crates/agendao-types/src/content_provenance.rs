use serde::{Deserialize, Serialize};

pub const EXTERNAL_CONTENT_PROVENANCE_METADATA_KEY: &str = "external_content_provenance";

/// Origin of content that entered the model context through a non-user,
/// non-system boundary. The classification describes trust, not permission:
/// it never grants execution and it never marks the content as malicious.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalContentSourceKind {
    UnknownExternal,
    Web,
    Mcp,
    Plugin,
    RemoteSkill,
    DynamicTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalContentProvenance {
    pub source_kind: ExternalContentSourceKind,
    pub resource_id: String,
    pub fetched_at: i64,
    /// External content may contain instructions addressed to the model. It
    /// remains data and must pass ordinary tool schema, permission, and
    /// workspace authority before any requested action can execute.
    pub untrusted_external: bool,
}

impl ExternalContentProvenance {
    pub fn untrusted(
        source_kind: ExternalContentSourceKind,
        resource_id: impl Into<String>,
        fetched_at: i64,
    ) -> Self {
        Self {
            source_kind,
            resource_id: resource_id.into(),
            fetched_at,
            untrusted_external: true,
        }
    }

    pub fn from_metadata(
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Option<Self> {
        Self::all_from_metadata(metadata).into_iter().next()
    }

    pub fn all_from_metadata(
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Vec<Self> {
        let Some(value) = metadata.get(EXTERNAL_CONTENT_PROVENANCE_METADATA_KEY) else {
            return Vec::new();
        };
        match value {
            serde_json::Value::Array(items) => items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect(),
            value => serde_json::from_value(value.clone()).into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_round_trip_keeps_untrusted_authority() {
        let value = serde_json::to_value(ExternalContentProvenance::untrusted(
            ExternalContentSourceKind::Mcp,
            "server/resource",
            42,
        ))
        .unwrap();
        let restored: ExternalContentProvenance = serde_json::from_value(value).unwrap();
        assert!(restored.untrusted_external);
        assert_eq!(restored.resource_id, "server/resource");
    }

    #[test]
    fn metadata_reader_accepts_multiple_mcp_resources() {
        let first =
            ExternalContentProvenance::untrusted(ExternalContentSourceKind::Mcp, "server/one", 1);
        let second =
            ExternalContentProvenance::untrusted(ExternalContentSourceKind::Mcp, "server/two", 2);
        let metadata = std::collections::HashMap::from([(
            EXTERNAL_CONTENT_PROVENANCE_METADATA_KEY.to_string(),
            serde_json::json!([first, second]),
        )]);
        let restored = ExternalContentProvenance::all_from_metadata(&metadata);
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[1].resource_id, "server/two");
    }
}
