use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceSkillArtifactVersion {
    #[serde(rename = "rocode-rust/workspace-skill/v1")]
    RocodeRustWorkspaceSkillV1,
}

impl WorkspaceSkillArtifactVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RocodeRustWorkspaceSkillV1 => "rocode-rust/workspace-skill/v1",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactRequiredEnvironmentVariable {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_for: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactHermesMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_skills: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactRocodeMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_for_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_toolsets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_for_toolsets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_filter: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactMetadataBlocks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hermes: Option<WorkspaceSkillArtifactHermesMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rocode: Option<WorkspaceSkillArtifactRocodeMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactPrerequisites {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_vars: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<WorkspaceSkillArtifactPrerequisites>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_environment_variables: Vec<WorkspaceSkillArtifactRequiredEnvironmentVariable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<WorkspaceSkillArtifactMetadataBlocks>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactFile {
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactEntry {
    pub frontmatter: WorkspaceSkillArtifactFrontmatter,
    pub body: String,
    #[serde(default)]
    pub supporting_files: Vec<WorkspaceSkillArtifactFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactBundle {
    pub version: WorkspaceSkillArtifactVersion,
    pub exported_at: i64,
    #[serde(default)]
    pub skills: Vec<WorkspaceSkillArtifactEntry>,
}

impl WorkspaceSkillArtifactBundle {
    pub fn new(exported_at: i64, skills: Vec<WorkspaceSkillArtifactEntry>) -> Self {
        Self {
            version: WorkspaceSkillArtifactVersion::RocodeRustWorkspaceSkillV1,
            exported_at,
            skills,
        }
    }

    pub fn new_now(skills: Vec<WorkspaceSkillArtifactEntry>) -> Self {
        Self::new(Utc::now().timestamp_millis(), skills)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum WorkspaceSkillArtifactImportEnvelope {
    Bundle(WorkspaceSkillArtifactBundle),
    Legacy(WorkspaceSkillArtifactLegacyPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSkillArtifactLegacyPayload {
    pub legacy_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::{
        WorkspaceSkillArtifactBundle, WorkspaceSkillArtifactEntry, WorkspaceSkillArtifactFile,
        WorkspaceSkillArtifactFrontmatter, WorkspaceSkillArtifactImportEnvelope,
        WorkspaceSkillArtifactVersion,
    };

    fn sample_entry() -> WorkspaceSkillArtifactEntry {
        WorkspaceSkillArtifactEntry {
            frontmatter: WorkspaceSkillArtifactFrontmatter {
                name: "reviewer".to_string(),
                description: "Review code changes".to_string(),
                version: Some("1.0.0".to_string()),
                tags: vec!["review".to_string()],
                required_commands: vec!["cargo".to_string()],
                ..WorkspaceSkillArtifactFrontmatter::default()
            },
            body: "# Reviewer\n\nInspect patches carefully.".to_string(),
            supporting_files: vec![WorkspaceSkillArtifactFile {
                relative_path: "templates/checklist.md".to_string(),
                content: "- scope\n- tests\n".to_string(),
            }],
        }
    }

    #[test]
    fn bundle_serializes_with_stable_version_and_skills() {
        let bundle = WorkspaceSkillArtifactBundle::new(123, vec![sample_entry()]);

        let value = serde_json::to_value(&bundle).expect("bundle should serialize");

        assert_eq!(
            value["version"],
            serde_json::json!(WorkspaceSkillArtifactVersion::RocodeRustWorkspaceSkillV1.as_str())
        );
        assert_eq!(value["exported_at"], serde_json::json!(123));
        assert_eq!(value["skills"].as_array().map(Vec::len), Some(1));
        assert!(value.get("managed_skills").is_none());
        assert!(value.get("distributions").is_none());
    }

    #[test]
    fn bundle_roundtrips_through_import_envelope() {
        let bundle = WorkspaceSkillArtifactBundle::new(123, vec![sample_entry()]);

        let payload = serde_json::to_string(&bundle).expect("bundle should serialize");
        let envelope: WorkspaceSkillArtifactImportEnvelope =
            serde_json::from_str(&payload).expect("bundle should parse");

        match envelope {
            WorkspaceSkillArtifactImportEnvelope::Bundle(parsed) => {
                assert_eq!(parsed.exported_at, 123);
                assert_eq!(parsed.skills.len(), 1);
                assert_eq!(parsed.skills[0].frontmatter.name, "reviewer");
                assert_eq!(
                    parsed.skills[0].supporting_files[0].relative_path,
                    "templates/checklist.md"
                );
            }
            WorkspaceSkillArtifactImportEnvelope::Legacy(_) => panic!("expected bundle envelope"),
        }
    }

    #[test]
    fn import_envelope_rejects_unknown_bundle_version() {
        let payload = serde_json::json!({
            "version": "rocode-rust/workspace-skill/v999",
            "exported_at": 123,
            "skills": [sample_entry()]
        });

        let error = serde_json::from_value::<WorkspaceSkillArtifactImportEnvelope>(payload)
            .expect_err("unknown version should fail closed");
        assert!(
            error.to_string().contains("did not match any variant")
                || error.to_string().contains("unknown variant")
        );
    }

    #[test]
    fn import_envelope_accepts_only_explicit_legacy_shape() {
        let payload = serde_json::json!({
            "legacy_format": "workspace-skill-alpha",
            "payload": {
                "skills": [{"name": "legacy-reviewer"}]
            }
        });

        let envelope: WorkspaceSkillArtifactImportEnvelope =
            serde_json::from_value(payload).expect("explicit legacy shape should parse");

        match envelope {
            WorkspaceSkillArtifactImportEnvelope::Legacy(legacy) => {
                assert_eq!(legacy.legacy_format, "workspace-skill-alpha");
                assert!(legacy.payload.is_some());
            }
            WorkspaceSkillArtifactImportEnvelope::Bundle(_) => panic!("expected legacy envelope"),
        }
    }

    #[test]
    fn import_envelope_rejects_unknown_bundle_fields() {
        let payload = serde_json::json!({
            "version": "rocode-rust/workspace-skill/v1",
            "exported_at": 123,
            "skills": [sample_entry()],
            "managed_skills": []
        });

        let error = serde_json::from_value::<WorkspaceSkillArtifactImportEnvelope>(payload)
            .expect_err("unknown bundle fields should fail closed");
        assert!(!error.to_string().is_empty());
    }
}
