use chrono::{DateTime, Utc};
use std::{fs, path::{Path, PathBuf}};

use crate::file_validation::hash::hash_file;
use crate::metadata_manager::{Metadata, MetadataError, MetadataManager};

pub struct MetadataHandler;

impl MetadataHandler {
    pub fn collect_file(path: impl AsRef<Path>) -> Result<Metadata, MetadataError> {
        let path = path.as_ref();

        let canonical_path = fs::canonicalize(path)?;
        let metadata = fs::metadata(&canonical_path)?;
        if metadata.is_dir() {
            return Err(MetadataError::InvalidInput(
                "Directories are not supported for file metadata collection".into(),
            ));
        }

        let modified_at = metadata.modified()?;
        let modified_at: DateTime<Utc> = modified_at.into();

        let file_size = metadata.len();
        let file_str = canonical_path
            .to_str()
            .ok_or_else(|| MetadataError::InvalidInput("Path is not valid UTF-8".into()))?;

        let sha256 = hash_file(file_str)?;

        Ok(Metadata::new(canonical_path, file_size, sha256, modified_at))
    }

    pub fn sync_file_metadata(
        path: impl AsRef<Path>,
        metadata_path: impl Into<PathBuf>,
    ) -> Result<Metadata, MetadataError> {
        let metadata = Self::collect_file(path)?;
        let manager = MetadataManager::new(metadata_path.into());
        manager.update_metadata(metadata.clone())?;
        Ok(metadata)
    }
}
