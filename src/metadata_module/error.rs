use thiserror::Error;
use std::path::PathBuf;

#[derive(Error, Debug)]
pub enum MetadataError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Metadata file not found at: {0}")]
    NotFound(PathBuf),

    #[error("Invalid metadata input: {0}")]
    InvalidInput(String),
}
