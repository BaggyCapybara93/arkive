use thiserror::Error;

//Error Handling
#[derive(Error, Debug)]
pub enum FileManagerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hash mismatch after copy — file may be corrupted")]
    HashMismatch,

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Invalid directory: {0}")]
    InvalidDirectory(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}