use crate::batch_module::BatchError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("File operation failed: {0}")]
    FileError(#[from] std::io::Error),

    #[error("Batch error: {0}")]
    BatchError(#[from] BatchError),

    #[error("File manager error: {0}")]
    FileManager(#[from] crate::file_module::FileManagerError),

    #[error("Config error: {0}")]
    ConfigError(#[from] crate::config_module::ConfigError),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}
