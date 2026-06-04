use agendao_storage_core::{
    repository::SessionRepository, SessionRecord, StorageError, StorageResult,
};
use sqlx::sqlite::SqlitePool;

#[derive(Clone)]
pub struct SqliteSessionRepository {
    pool: SqlitePool,
}

impl SqliteSessionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl SessionRepository for SqliteSessionRepository {
    async fn get(&self, _session_id: &str) -> StorageResult<Option<SessionRecord>> {
        Err(StorageError::Unimplemented("sqlite session get"))
    }

    async fn upsert(&self, _session: &SessionRecord) -> StorageResult<()> {
        Err(StorageError::Unimplemented("sqlite session upsert"))
    }

    async fn list_recent(&self, _limit: u32) -> StorageResult<Vec<SessionRecord>> {
        Err(StorageError::Unimplemented("sqlite session list_recent"))
    }
}
