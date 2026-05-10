    use super::*;
    use crate::catalog::{
        snapshot_path, StoredSkillCatalogSnapshot, SKILL_CATALOG_SNAPSHOT_SCHEMA,
        SKILL_CATALOG_SNAPSHOT_VERSION,
    };
    use rocode_config::{Config, ConfigStore, SkillHubConfig, SkillsConfig};
    use rocode_types::{
        BundledSkillManifest, BundledSkillManifestEntry, ManagedSkillRecord,
        SkillArtifactCacheEntry, SkillArtifactCacheStatus, SkillArtifactKind, SkillArtifactRef,
        SkillAuditEvent, SkillAuditKind, SkillCapabilityGroup, SkillCapabilityGroupKind,
        SkillCapabilityGroupState, SkillCapabilityMember, SkillCapabilityMemberRole,
        SkillDistributionRecord, SkillDistributionRelease, SkillDistributionResolution,
        SkillDistributionResolverKind, SkillGovernanceTimelineKind, SkillInstalledDistribution,
        SkillManagedLifecycleRecord, SkillManagedLifecycleState, SkillOperationalSnapshot,
        SkillOperationalSourceScope, SkillRelationshipEdge, SkillRelationshipKind,
        SkillRelationshipState, SkillSourceKind, SkillSourceRef,
    };
    use sha2::Digest;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;

    fn write_directory_skill(
        root: &Path,
        relative_dir: &str,
        name: &str,
        description: &str,
        body: &str,
        supporting_files: &[(&str, &str)],
    ) {
        let skill_dir = root.join(relative_dir);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                r#"---
name: {name}
description: {description}
---
{body}
"#
            ),
        )
        .unwrap();
        for (relative_path, content) in supporting_files {
            let file_path = skill_dir.join(relative_path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(file_path, content).unwrap();
        }
    }

    #[path = "inspection.rs"]
    mod inspection;
    #[path = "governance.rs"]
    mod governance;

    #[path = "distribution.rs"]
    mod distribution;
