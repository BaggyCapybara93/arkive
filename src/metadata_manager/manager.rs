
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json;
use std::{fs, path::{Path, PathBuf}};

use crate::metadata_manager::error::MetadataError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub file_path: PathBuf,
    pub file_size: u64,
    pub sha256: String,

    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: DateTime<Utc>,

    #[serde(with = "chrono::serde::ts_seconds")]
    pub modified_at: DateTime<Utc>,

    #[serde(with = "chrono::serde::ts_seconds")]
    pub updated_at: DateTime<Utc>,
}

impl Metadata {
    pub fn new(file_path: PathBuf, file_size: u64, sha256: String, modified_at: DateTime<Utc>) -> Self {
        let now = Utc::now();

        Metadata {
            file_path,
            file_size,
            sha256,
            created_at: now,
            modified_at,
            updated_at: now,
        }
    }
}

pub struct MetadataManager {
    pub metadata_path: PathBuf,
}

impl MetadataManager {
    pub fn new(metadata_path: PathBuf) -> Self {
        MetadataManager { metadata_path }
    }

    pub fn load_all_metadata(&self) -> Result<Vec<Metadata>, MetadataError> {
        if !self.metadata_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.metadata_path)?;
        let metadata = serde_json::from_str(&content)?;

        Ok(metadata)
    }

    pub fn save_all_metadata(&self, metadata: &[Metadata]) -> Result<(), MetadataError> {
        if let Some(parent) = self.metadata_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(metadata)?;
        fs::write(&self.metadata_path, content)?;

        Ok(())
    }

    pub fn contains_path(&self, path: &Path) -> Result<bool, MetadataError> {
        let canonical_path = fs::canonicalize(path)?;
        let entries = self.load_all_metadata()?;

        Ok(entries.into_iter().any(|entry| entry.file_path == canonical_path))
    }

    pub fn find_metadata(&self, path: &Path) -> Result<Option<Metadata>, MetadataError> {
        let canonical_path = fs::canonicalize(path)?;
        let entries = self.load_all_metadata()?;

        Ok(entries.into_iter().find(|entry| entry.file_path == canonical_path))
    }

    pub fn update_metadata(&self, metadata: Metadata) -> Result<(), MetadataError> {
        let mut entries = self.load_all_metadata()?;
        let canonical_path = fs::canonicalize(&metadata.file_path)?;

        if let Some(existing) = entries.iter_mut().find(|entry| entry.file_path == canonical_path) {
            existing.file_size = metadata.file_size;
            existing.sha256 = metadata.sha256.clone();
            existing.modified_at = metadata.modified_at;
            existing.updated_at = Utc::now();
        } else {
            entries.push(Metadata {
                file_path: canonical_path,
                file_size: metadata.file_size,
                sha256: metadata.sha256.clone(),
                created_at: metadata.created_at,
                modified_at: metadata.modified_at,
                updated_at: Utc::now(),
            });
        }

        self.save_all_metadata(&entries)
    }
}
