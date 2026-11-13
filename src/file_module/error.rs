use std::path::PathBuf;
use thiserror::Error;

//Error Handling
#[derive(Error, Debug)]
pub enum FileManagerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("I/O error while {operation} at {path}: {source}", path = path.display())]
    IoAt {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "Failed to {operation} at {path:?}: {original}; rollback also failed: {rollback}. Check the displaced destination at {displaced:?}"
    )]
    RollbackFailed {
        operation: &'static str,
        path: PathBuf,
        displaced: PathBuf,
        #[source]
        original: std::io::Error,
        rollback: std::io::Error,
    },

    #[error("Hash mismatch after copy — file may be corrupted")]
    HashMismatch,

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Invalid directory: {0}")]
    InvalidDirectory(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}
