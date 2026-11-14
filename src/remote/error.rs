use thiserror::Error;

use crate::error::AppError;

#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("{0}")]
    Invalid(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub(super) fn invalid(message: impl Into<String>) -> AppError {
    RemoteError::Invalid(message.into()).into()
}

pub(super) fn protocol(message: impl Into<String>) -> AppError {
    RemoteError::Protocol(message.into()).into()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn io(error: rustix::io::Errno) -> AppError {
    RemoteError::Io(std::io::Error::from(error)).into()
}
