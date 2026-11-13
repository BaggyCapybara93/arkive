use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::metadata_module::{error::MetadataError, structs::Metadata};

/// Represents the contents of a single shard file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShardData {
    pub entries: Vec<Metadata>,
}

/// Manages a single shard file
pub struct LocalMetadataManager {
    shard_path: PathBuf,
}

impl LocalMetadataManager {
    pub fn new(shard_path: PathBuf) -> Self {
        Self { shard_path }
    }

    ///Loading and Saving
    pub fn load(&self) -> Result<ShardData, MetadataError> {
        let data = match fs::read_to_string(&self.shard_path) {
            Ok(data) => data,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ShardData::default());
            }
            Err(source) => {
                return Err(MetadataError::io(
                    "read metadata shard",
                    &self.shard_path,
                    source,
                ));
            }
        };
        let shard_data: ShardData =
            serde_json::from_str(&data).map_err(|source| MetadataError::CorruptShard {
                path: self.shard_path.clone(),
                source,
            })?;
        Ok(shard_data)
    }

    pub fn save(&self, shard_data: &ShardData) -> Result<(), MetadataError> {
        if let Some(parent) = self.shard_path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                MetadataError::io("create metadata shard directory", parent, source)
            })?;
        }

        if shard_data.entries.is_empty() {
            match fs::remove_file(&self.shard_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(MetadataError::io(
                        "remove empty metadata shard",
                        &self.shard_path,
                        source,
                    ));
                }
            }
            return Ok(());
        }

        let data = serde_json::to_vec(shard_data).map_err(|source| {
            MetadataError::json("serialize metadata shard", &self.shard_path, source)
        })?;
        Self::atomic_write_file(&self.shard_path, &data)?;
        Ok(())
    }

    fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<(), MetadataError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                MetadataError::io("create metadata shard directory", parent, source)
            })?;
        }

        let temp_path = Self::temp_path(path)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|source| {
                MetadataError::io("create temporary metadata shard", &temp_path, source)
            })?;

        if let Err(source) = output.write_all(contents) {
            return Err(Self::cleanup_after_failure(
                "write temporary metadata shard",
                path,
                &temp_path,
                MetadataError::io("write temporary metadata shard", &temp_path, source),
            ));
        }
        if let Err(source) = output.flush() {
            return Err(Self::cleanup_after_failure(
                "flush temporary metadata shard",
                path,
                &temp_path,
                MetadataError::io("flush temporary metadata shard", &temp_path, source),
            ));
        }
        if let Err(source) = output.sync_all() {
            return Err(Self::cleanup_after_failure(
                "sync temporary metadata shard",
                path,
                &temp_path,
                MetadataError::io("sync temporary metadata shard", &temp_path, source),
            ));
        }
        drop(output);

        match fs::rename(&temp_path, path) {
            Ok(()) => Ok(()),
            Err(source) => Err(Self::cleanup_after_failure(
                "publish metadata shard",
                path,
                &temp_path,
                MetadataError::io("publish metadata shard", path, source),
            )),
        }
    }

    fn cleanup_after_failure(
        operation: &'static str,
        path: &Path,
        temp_path: &Path,
        original: MetadataError,
    ) -> MetadataError {
        match fs::remove_file(temp_path) {
            Ok(()) => original,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => original,
            Err(source) => MetadataError::transaction_rollback(
                operation,
                path,
                original,
                MetadataError::io("remove temporary metadata shard", temp_path, source),
            ),
        }
    }

    fn temp_path(path: &Path) -> Result<PathBuf, MetadataError> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "shard.tmp".to_string());
        let temp_name = format!(".{file_name}.tmp.{timestamp}");
        Ok(path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(temp_name))
    }

    ///Operations
    pub fn upsert(&self, metadata: Metadata) -> Result<(), MetadataError> {
        let mut shard = self.load()?;

        if let Some(existing) = shard
            .entries
            .iter_mut()
            .find(|e| e.file_path == metadata.file_path)
        {
            *existing = metadata;
        } else {
            shard.entries.push(metadata);
        }

        self.save(&shard)
    }

    pub fn get(&self, canonical_path: &Path) -> Result<Option<Metadata>, MetadataError> {
        let shard = self.load()?;
        Ok(shard
            .entries
            .into_iter()
            .find(|e| e.file_path == canonical_path))
    }

    pub fn remove(&self, canonical_path: &Path) -> Result<bool, MetadataError> {
        let mut shard = self.load()?;
        let before = shard.entries.len();

        shard.entries.retain(|e| e.file_path != canonical_path);

        let removed = shard.entries.len() != before;

        if removed {
            self.save(&shard)?;
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata_module::error::MetadataError;
    use crate::test::TestDir;

    #[test]
    fn invalid_shard_reports_corruption_and_its_path() {
        let temp = TestDir::new("metadata-invalid-shard");
        let shard_path = temp.path().join("shard.json");
        fs::write(&shard_path, b"not json").unwrap();
        let manager = LocalMetadataManager::new(shard_path.clone());

        let error = manager.load().unwrap_err();
        match error {
            MetadataError::CorruptShard { path, .. } => assert_eq!(path, shard_path),
            other => panic!("expected corrupt shard error, got {other:?}"),
        }
    }

    #[test]
    fn unreadable_shard_reports_an_io_error_instead_of_corruption() {
        let temp = TestDir::new("metadata-shard-io-error");
        let shard_path = temp.path().join("shard.json");
        fs::create_dir(&shard_path).unwrap();
        let manager = LocalMetadataManager::new(shard_path.clone());

        let error = manager.load().unwrap_err();
        match error {
            MetadataError::Io {
                operation,
                path,
                source,
            } => {
                assert_eq!(operation, "read metadata shard");
                assert_eq!(path, shard_path);
                assert_eq!(source.kind(), std::io::ErrorKind::IsADirectory);
            }
            other => panic!("expected metadata shard io error, got {other:?}"),
        }
    }

    #[test]
    fn save_reports_the_parent_path_when_it_cannot_create_a_shard_directory() {
        let temp = TestDir::new("metadata-shard-save-error");
        let parent = temp.path().join("not-a-directory");
        fs::write(&parent, b"file").unwrap();
        let shard_path = parent.join("shard.json");
        let manager = LocalMetadataManager::new(shard_path);

        let error = manager.save(&ShardData::default()).unwrap_err();
        match error {
            MetadataError::Io {
                operation, path, ..
            } => {
                assert_eq!(operation, "create metadata shard directory");
                assert_eq!(path, parent);
            }
            other => panic!("expected metadata shard save error, got {other:?}"),
        }
    }
}
