use agendao_storage_core::{StorageBackend, StorageError, StorageResult};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

use crate::repository::{
    SqliteArtifactRepository, SqliteMessageRepository, SqliteSessionRepository,
};

#[derive(Debug, Clone)]
pub struct SqliteConfig {
    pub url: String,
    pub max_connections: u32,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            url: "sqlite::memory:".to_string(),
            max_connections: 1,
        }
    }
}

#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
    sessions: SqliteSessionRepository,
    messages: SqliteMessageRepository,
    artifacts: SqliteArtifactRepository,
}

impl SqliteStorage {
    pub async fn connect(config: &SqliteConfig) -> StorageResult<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&config.url)
            .await
            .map_err(|error| StorageError::Backend(error.to_string()))?;

        Ok(Self::from_pool(pool))
    }

    pub fn from_pool(pool: SqlitePool) -> Self {
        Self {
            sessions: SqliteSessionRepository::new(pool.clone()),
            messages: SqliteMessageRepository::new(pool.clone()),
            artifacts: SqliteArtifactRepository::new(pool.clone()),
            pool,
        }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl StorageBackend for SqliteStorage {
    type Sessions = SqliteSessionRepository;
    type Messages = SqliteMessageRepository;
    type Artifacts = SqliteArtifactRepository;

    async fn healthcheck(&self) -> StorageResult<()> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|error| StorageError::Backend(error.to_string()))
    }

    fn sessions(&self) -> &Self::Sessions {
        &self.sessions
    }

    fn messages(&self) -> &Self::Messages {
        &self.messages
    }

    fn artifacts(&self) -> &Self::Artifacts {
        &self.artifacts
    }
}
