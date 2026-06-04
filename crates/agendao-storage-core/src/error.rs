use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage backend error: {0}")]
    Backend(String),

    #[error("storage operation is not implemented: {0}")]
    Unimplemented(&'static str),
}

pub type StorageResult<T> = Result<T, StorageError>;
