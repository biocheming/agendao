use agendao_storage_core::{
    repository::MessageRepository, MessageRecord, StorageError, StorageResult,
};
use sqlx::sqlite::SqlitePool;

#[derive(Clone)]
pub struct SqliteMessageRepository {
    pool: SqlitePool,
}

impl SqliteMessageRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl MessageRepository for SqliteMessageRepository {
    async fn list_for_session(&self, _session_id: &str) -> StorageResult<Vec<MessageRecord>> {
        Err(StorageError::Unimplemented(
            "sqlite message list_for_session",
        ))
    }

    async fn append(&self, _message: &MessageRecord) -> StorageResult<()> {
        Err(StorageError::Unimplemented("sqlite message append"))
    }
}
