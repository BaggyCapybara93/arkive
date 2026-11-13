use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MetadataError {
    #[error("I/O error while {operation} at {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("JSON error while {operation} at {path}: {source}")]
    Json {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Invalid metadata input: {0}")]
    InvalidInput(String),

    #[error("Corrupt index file at {path}: {source}")]
    CorruptIndex {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Corrupted shard data at {path}: {source}")]
    CorruptShard {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "Metadata transaction failed while {operation} at {path}; original error: {original}; rollback error: {rollback}"
    )]
    TransactionRollback {
        operation: &'static str,
        path: PathBuf,
        original: Box<Self>,
        rollback: Box<Self>,
    },

    #[error("Path error: {0}")]
    PathError(String),
}

impl MetadataError {
    pub(crate) fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }

    pub(crate) fn json(
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: serde_json::Error,
    ) -> Self {
        Self::Json {
            operation,
            path: path.into(),
            source,
        }
    }

    pub(crate) fn transaction_rollback(
        operation: &'static str,
        path: impl Into<PathBuf>,
        original: Self,
        rollback: Self,
    ) -> Self {
        Self::TransactionRollback {
            operation,
            path: path.into(),
            original: Box::new(original),
            rollback: Box::new(rollback),
        }
    }
}
