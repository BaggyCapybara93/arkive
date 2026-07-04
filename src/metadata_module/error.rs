use thiserror::Error;

#[derive(Error, Debug)]
pub enum MetadataError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid metadata input: {0}")]
    InvalidInput(String),

    #[error("Corrupt index file: {0}")]
    CorruptIndex(String),

    #[error("Corrupted shard data: {0}")]
    CorruptShard(String),
}
