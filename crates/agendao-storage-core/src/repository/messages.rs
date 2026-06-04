use async_trait::async_trait;

use crate::{models::MessageRecord, StorageResult};

#[async_trait]
pub trait MessageRepository: Send + Sync {
    async fn list_for_session(&self, session_id: &str) -> StorageResult<Vec<MessageRecord>>;

    async fn append(&self, message: &MessageRecord) -> StorageResult<()>;
}
