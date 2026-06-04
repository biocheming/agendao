use async_trait::async_trait;

use crate::{models::SessionRecord, StorageResult};

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn get(&self, session_id: &str) -> StorageResult<Option<SessionRecord>>;

    async fn upsert(&self, session: &SessionRecord) -> StorageResult<()>;

    async fn list_recent(&self, limit: u32) -> StorageResult<Vec<SessionRecord>>;
}
