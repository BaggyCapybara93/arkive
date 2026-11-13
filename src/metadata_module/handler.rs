use chrono::{DateTime, Utc};
use std::{fs, path::Path};

use crate::file_validation::hash::hash_file;
use crate::metadata_module::MetadataError;
use crate::metadata_module::structs::Metadata;

pub struct MetadataHandler;

impl MetadataHandler {
    pub fn collect_file(path: impl AsRef<Path>) -> Result<Metadata, MetadataError> {
        let path = path.as_ref();

        let canonical_path = fs::canonicalize(path)
            .map_err(|source| MetadataError::io("canonicalize metadata source", path, source))?;
        let metadata = fs::metadata(&canonical_path)
            .map_err(|source| MetadataError::io("read metadata source", &canonical_path, source))?;
        if metadata.is_dir() {
            return Err(MetadataError::InvalidInput(
                "Directories are not supported for file metadata collection".into(),
            ));
        }

        let modified_at = metadata.modified().map_err(|source| {
            MetadataError::io("read metadata modification time", &canonical_path, source)
        })?;
        let modified_at: DateTime<Utc> = modified_at.into();

        let file_size = metadata.len();
        let file_str = canonical_path
            .to_str()
            .ok_or_else(|| MetadataError::InvalidInput("Path is not valid UTF-8".into()))?;

        let sha256 = hash_file(file_str)
            .map_err(|source| MetadataError::io("hash metadata source", &canonical_path, source))?;

        Ok(Metadata::new(
            canonical_path,
            file_size,
            sha256,
            modified_at,
        ))
    }
}
