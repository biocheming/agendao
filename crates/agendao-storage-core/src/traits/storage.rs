use async_trait::async_trait;

use crate::{
    repository::{ArtifactRepository, MessageRepository, SessionRepository},
    StorageResult,
};

#[async_trait]
pub trait StorageBackend: Send + Sync {
    type Sessions: SessionRepository;
    type Messages: MessageRepository;
    type Artifacts: ArtifactRepository;

    async fn healthcheck(&self) -> StorageResult<()>;

    fn sessions(&self) -> &Self::Sessions;

    fn messages(&self) -> &Self::Messages;

    fn artifacts(&self) -> &Self::Artifacts;
}
