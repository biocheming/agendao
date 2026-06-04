use agendao_storage_core::{
    repository::ArtifactRepository, ArtifactRecord, StorageError, StorageResult,
};
use sqlx::sqlite::SqlitePool;

#[derive(Clone)]
pub struct SqliteArtifactRepository {
    pool: SqlitePool,
}

impl SqliteArtifactRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl ArtifactRepository for SqliteArtifactRepository {
    async fn list_for_session(&self, _session_id: &str) -> StorageResult<Vec<ArtifactRecord>> {
        Err(StorageError::Unimplemented(
            "sqlite artifact list_for_session",
        ))
    }

    async fn insert(&self, _artifact: &ArtifactRecord) -> StorageResult<()> {
        Err(StorageError::Unimplemented("sqlite artifact insert"))
    }
}
