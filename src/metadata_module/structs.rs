use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents metadata for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub file_path: PathBuf,
    pub file_size: u64,
    pub sha256: String,

    #[serde(with = "chrono::serde::ts_seconds")]
    pub modified_at: DateTime<Utc>,

    #[serde(with = "chrono::serde::ts_seconds")]
    pub updated_at: DateTime<Utc>,
}

impl Metadata {
    pub fn new(
        file_path: PathBuf,
        file_size: u64,
        sha256: String,
        modified_at: DateTime<Utc>,
    ) -> Self {
        Self {
            file_path,
            file_size,
            sha256,
            modified_at,
            updated_at: modified_at,
        }
    }
}
