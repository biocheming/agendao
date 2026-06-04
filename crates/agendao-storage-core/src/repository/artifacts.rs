use async_trait::async_trait;

use crate::{models::ArtifactRecord, StorageResult};

#[async_trait]
pub trait ArtifactRepository: Send + Sync {
    async fn list_for_session(&self, session_id: &str) -> StorageResult<Vec<ArtifactRecord>>;

    async fn insert(&self, artifact: &ArtifactRecord) -> StorageResult<()>;
}
