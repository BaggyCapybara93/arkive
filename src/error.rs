use thiserror::Error;
use crate::batch_handler::BatchError;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("File operation failed: {0}")]
    FileError(#[from] std::io::Error),

    #[error("Batch error: {0}")]
    BatchError(#[from] BatchError),

    #[error("File manager error: {0}")]
    FileManager(#[from] crate::file_manager::FileManagerError),

    #[error("Unexpected error: {0}")]
    Other(String),
}