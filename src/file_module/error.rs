use thiserror::Error;

//Error Handling
#[derive(Error, Debug)]
pub enum FileManagerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hash mismatch after copy — file may be corrupted")]
    HashMismatch,

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}