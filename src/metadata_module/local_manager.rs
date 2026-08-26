use serde::{Deserialize, Serialize};
use std::{
    fs,
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
        if !self.shard_path.exists() {
            return Ok(ShardData::default());
        }

        let data = fs::read_to_string(&self.shard_path).map_err(|e| {
            MetadataError::CorruptShard(format!("Failed to read shard file: {}", e))
        })?;
        let shard_data: ShardData = serde_json::from_str(&data).map_err(|e| {
            MetadataError::CorruptShard(format!("Failed to parse shard file: {}", e))
        })?;
        Ok(shard_data)
    }

    pub fn save(&self, shard_data: &ShardData) -> Result<(), MetadataError> {
        if let Some(parent) = self.shard_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if shard_data.entries.is_empty() {
            if self.shard_path.exists() {
                fs::remove_file(&self.shard_path)?;
            }
            return Ok(());
        }

        let data = serde_json::to_vec(shard_data)?;
        Self::atomic_write_file(&self.shard_path, &data)?;
        Ok(())
    }

    fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<(), MetadataError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let temp_path = Self::temp_path(path)?;
        fs::write(&temp_path, contents)?;

        match fs::rename(&temp_path, path) {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = fs::remove_file(&temp_path);
                Err(MetadataError::Io(err))
            }
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

        self.save(&shard).map_err(|e| {
            MetadataError::CorruptShard(format!("Failed to save shard after upsert: {}", e))
        })
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
            self.save(&shard).map_err(|e| {
                MetadataError::CorruptShard(format!("Failed to save shard after remove: {}", e))
            })?;
        }

        Ok(removed)
    }
}
