use std::collections::HashSet;

use anyhow::Result;

use rocode_storage::{MemoryRepository, MemoryRepositoryFilter};
use rocode_types::{
    MemoryArtifactBundle, MemoryArtifactImportEnvelope, MemoryArtifactLegacyPayload, MemoryRecord,
};

pub trait MemoryArtifactLegacyAdapter {
    fn legacy_format(&self) -> &'static str;

    fn import_records(&self, payload: &MemoryArtifactLegacyPayload) -> Result<Vec<MemoryRecord>>;
}

pub async fn export_memory_artifact_bundle(
    memory_repo: &MemoryRepository,
) -> Result<MemoryArtifactBundle> {
    // Export is an authority snapshot, not a paginated read model. It must not
    // inherit repository defaults that cap interactive listings.
    let mut records = memory_repo
        .list_records(Some(&MemoryRepositoryFilter {
            limit: Some(i64::MAX),
            ..MemoryRepositoryFilter::default()
        }))
        .await?;
    records.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    Ok(MemoryArtifactBundle::new_now(records))
}

pub async fn import_memory_artifact_bundle(
    memory_repo: &MemoryRepository,
    payload: MemoryArtifactImportEnvelope,
) -> Result<usize> {
    import_memory_artifact_bundle_with_legacy_adapter(memory_repo, payload, None).await
}

pub async fn import_memory_artifact_bundle_with_legacy_adapter(
    memory_repo: &MemoryRepository,
    payload: MemoryArtifactImportEnvelope,
    legacy_adapter: Option<&dyn MemoryArtifactLegacyAdapter>,
) -> Result<usize> {
    let records = resolve_records_from_artifact(payload, legacy_adapter)?;
    validate_memory_records(&records)?;

    for record in &records {
        memory_repo.upsert_record(record).await?;
    }

    Ok(records.len())
}

fn resolve_records_from_artifact(
    payload: MemoryArtifactImportEnvelope,
    legacy_adapter: Option<&dyn MemoryArtifactLegacyAdapter>,
) -> Result<Vec<MemoryRecord>> {
    match payload {
        MemoryArtifactImportEnvelope::Bundle(bundle) => Ok(bundle.records),
        MemoryArtifactImportEnvelope::Legacy(legacy) => match legacy_adapter {
            Some(adapter) if adapter.legacy_format() == legacy.legacy_format => {
                adapter.import_records(&legacy)
            }
            _ => unsupported_legacy_format(&legacy.legacy_format),
        },
    }
}

fn unsupported_legacy_format(format: &str) -> Result<Vec<MemoryRecord>> {
    anyhow::bail!(
        "Unsupported legacy memory artifact format: {} (explicit legacy adapter required)",
        format
    );
}

fn validate_memory_records(records: &[MemoryRecord]) -> Result<()> {
    let mut seen = HashSet::new();
    for record in records {
        if !seen.insert(record.id.0.as_str()) {
            anyhow::bail!("Duplicate memory record id in artifact: {}", record.id.0);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        export_memory_artifact_bundle, import_memory_artifact_bundle,
        import_memory_artifact_bundle_with_legacy_adapter, MemoryArtifactLegacyAdapter,
    };
    use anyhow::Result;
    use rocode_storage::{Database, MemoryRepository};
    use rocode_types::{
        MemoryArtifactBundle, MemoryArtifactImportEnvelope, MemoryArtifactLegacyPayload,
        MemoryEvidenceRef, MemoryKind, MemoryRecord, MemoryRecordId, MemoryScope, MemoryStatus,
        MemoryValidationStatus,
    };

    struct AlphaLegacyAdapter;

    impl MemoryArtifactLegacyAdapter for AlphaLegacyAdapter {
        fn legacy_format(&self) -> &'static str {
            "memory-alpha"
        }

        fn import_records(
            &self,
            payload: &MemoryArtifactLegacyPayload,
        ) -> Result<Vec<MemoryRecord>> {
            #[derive(serde::Deserialize)]
            struct LegacyAlphaRecord {
                id: String,
                updated_at: i64,
            }

            #[derive(serde::Deserialize)]
            struct LegacyAlphaPayload {
                records: Vec<LegacyAlphaRecord>,
            }

            let raw = payload
                .payload
                .clone()
                .ok_or_else(|| anyhow::anyhow!("legacy payload body missing"))?;
            let parsed: LegacyAlphaPayload = serde_json::from_value(raw)?;
            Ok(parsed
                .records
                .into_iter()
                .map(|record| sample_record(&record.id, record.updated_at))
                .collect())
        }
    }

    fn sample_record(id: &str, updated_at: i64) -> MemoryRecord {
        MemoryRecord {
            id: MemoryRecordId(id.to_string()),
            kind: MemoryKind::Preference,
            scope: MemoryScope::WorkspaceShared,
            status: MemoryStatus::Validated,
            title: format!("title-{id}"),
            summary: format!("summary-{id}"),
            trigger_conditions: vec!["editing".to_string()],
            normalized_facts: vec![format!("fact-{id}")],
            boundaries: vec!["stay scoped".to_string()],
            confidence: Some(0.9),
            evidence_refs: vec![MemoryEvidenceRef {
                session_id: Some("session-1".to_string()),
                message_id: Some("message-1".to_string()),
                tool_call_id: None,
                stage_id: Some("stage-1".to_string()),
                note: Some("stated".to_string()),
            }],
            source_session_id: Some("session-1".to_string()),
            workspace_identity: Some("ws:test".to_string()),
            created_at: updated_at - 10,
            updated_at,
            last_validated_at: Some(updated_at),
            expires_at: None,
            derived_skill_name: None,
            linked_skill_name: Some("patch-review".to_string()),
            validation_status: MemoryValidationStatus::Passed,
        }
    }

    #[tokio::test]
    async fn export_memory_bundle_sorts_records_by_id() {
        let db = Database::in_memory().await.expect("db");
        let repo = MemoryRepository::new(db.pool().clone());
        repo.upsert_record(&sample_record("mem_z", 200))
            .await
            .expect("upsert");
        repo.upsert_record(&sample_record("mem_a", 100))
            .await
            .expect("upsert");

        let bundle = export_memory_artifact_bundle(&repo).await.expect("export");

        assert_eq!(bundle.records.len(), 2);
        assert_eq!(bundle.records[0].id.0, "mem_a");
        assert_eq!(bundle.records[1].id.0, "mem_z");
    }

    #[tokio::test]
    async fn export_memory_bundle_is_not_capped_by_default_repository_limit() {
        let db = Database::in_memory().await.expect("db");
        let repo = MemoryRepository::new(db.pool().clone());
        for index in 0..101 {
            repo.upsert_record(&sample_record(&format!("mem_{index:03}"), index))
                .await
                .expect("upsert");
        }

        let bundle = export_memory_artifact_bundle(&repo).await.expect("export");

        assert_eq!(bundle.records.len(), 101);
        assert_eq!(
            bundle.records.first().map(|record| record.id.0.as_str()),
            Some("mem_000")
        );
        assert_eq!(
            bundle.records.last().map(|record| record.id.0.as_str()),
            Some("mem_100")
        );
    }

    #[tokio::test]
    async fn import_memory_bundle_preserves_core_record_fields() {
        let db = Database::in_memory().await.expect("db");
        let repo = MemoryRepository::new(db.pool().clone());
        let record = sample_record("mem_1", 123);
        let envelope = MemoryArtifactImportEnvelope::Bundle(MemoryArtifactBundle::new(
            999,
            vec![record.clone()],
        ));

        let imported = import_memory_artifact_bundle(&repo, envelope)
            .await
            .expect("import should succeed");
        assert_eq!(imported, 1);

        let stored = repo
            .get_record("mem_1")
            .await
            .expect("query should succeed")
            .expect("record should exist");
        assert_eq!(stored.id, record.id);
        assert_eq!(stored.status, record.status);
        assert_eq!(stored.validation_status, record.validation_status);
        assert_eq!(stored.created_at, record.created_at);
        assert_eq!(stored.updated_at, record.updated_at);
        assert_eq!(stored.workspace_identity, record.workspace_identity);
        assert_eq!(stored.evidence_refs, record.evidence_refs);
    }

    #[tokio::test]
    async fn memory_artifact_roundtrips_through_parse_import_and_re_export() {
        let source_db = Database::in_memory().await.expect("db");
        let source_repo = MemoryRepository::new(source_db.pool().clone());
        source_repo
            .upsert_record(&sample_record("mem_b", 200))
            .await
            .expect("upsert");
        source_repo
            .upsert_record(&sample_record("mem_a", 100))
            .await
            .expect("upsert");

        let exported = export_memory_artifact_bundle(&source_repo)
            .await
            .expect("export should succeed");
        let payload = serde_json::to_string(&exported).expect("serialize");
        let parsed: MemoryArtifactImportEnvelope =
            serde_json::from_str(&payload).expect("parse should succeed");

        let target_db = Database::in_memory().await.expect("db");
        let target_repo = MemoryRepository::new(target_db.pool().clone());
        import_memory_artifact_bundle(&target_repo, parsed)
            .await
            .expect("import should succeed");
        let replayed = export_memory_artifact_bundle(&target_repo)
            .await
            .expect("re-export should succeed");

        assert_eq!(replayed.version, exported.version);
        assert_eq!(replayed.records, exported.records);
    }

    #[tokio::test]
    async fn import_memory_bundle_rejects_duplicate_record_ids() {
        let db = Database::in_memory().await.expect("db");
        let repo = MemoryRepository::new(db.pool().clone());
        let first = sample_record("mem_dup", 100);
        let second = sample_record("mem_dup", 200);
        let envelope = MemoryArtifactImportEnvelope::Bundle(MemoryArtifactBundle::new(
            999,
            vec![first, second],
        ));

        let error = import_memory_artifact_bundle(&repo, envelope)
            .await
            .expect_err("duplicate ids should fail");
        assert!(error.to_string().contains("Duplicate memory record id"));
    }

    #[tokio::test]
    async fn import_memory_bundle_rejects_legacy_payload_without_explicit_adapter() {
        let db = Database::in_memory().await.expect("db");
        let repo = MemoryRepository::new(db.pool().clone());
        let envelope =
            MemoryArtifactImportEnvelope::Legacy(rocode_types::MemoryArtifactLegacyPayload {
                legacy_format: "memory-alpha".to_string(),
                payload: Some(serde_json::json!({"records": []})),
            });

        let error = import_memory_artifact_bundle(&repo, envelope)
            .await
            .expect_err("legacy payload should fail closed");
        assert!(error
            .to_string()
            .contains("Unsupported legacy memory artifact format: memory-alpha"));
    }

    #[tokio::test]
    async fn import_memory_bundle_accepts_matching_explicit_legacy_adapter() {
        let db = Database::in_memory().await.expect("db");
        let repo = MemoryRepository::new(db.pool().clone());
        let envelope =
            MemoryArtifactImportEnvelope::Legacy(rocode_types::MemoryArtifactLegacyPayload {
                legacy_format: "memory-alpha".to_string(),
                payload: Some(serde_json::json!({
                    "records": [{"id": "mem_legacy", "updated_at": 321}]
                })),
            });

        let imported = import_memory_artifact_bundle_with_legacy_adapter(
            &repo,
            envelope,
            Some(&AlphaLegacyAdapter),
        )
        .await
        .expect("legacy adapter should import");
        assert_eq!(imported, 1);

        let stored = repo
            .get_record("mem_legacy")
            .await
            .expect("query should succeed")
            .expect("record should exist");
        assert_eq!(stored.id.0, "mem_legacy");
        assert_eq!(stored.updated_at, 321);
    }
}
