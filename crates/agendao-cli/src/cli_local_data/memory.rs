use agendao_memory::{export_memory_artifact_bundle, import_memory_artifact_bundle};
use agendao_storage::{Database, MemoryRepository};
use agendao_types::{MemoryArtifactBundle, MemoryArtifactImportEnvelope};

pub(crate) async fn export_memory_bundle() -> anyhow::Result<MemoryArtifactBundle> {
    let db = Database::new().await?;
    let memory_repo = MemoryRepository::new(db.pool().clone());
    Ok(export_memory_artifact_bundle(&memory_repo).await?)
}

pub(crate) async fn import_memory_bundle(
    payload: MemoryArtifactImportEnvelope,
) -> anyhow::Result<usize> {
    let db = Database::new().await?;
    let memory_repo = MemoryRepository::new(db.pool().clone());
    Ok(import_memory_artifact_bundle(&memory_repo, payload).await?)
}
