use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::metadata_module::{
    error::MetadataError,
    structs::Metadata,
};

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

        let data = fs::read_to_string(&self.shard_path)?;
        let shard_data: ShardData = serde_json::from_str(&data)?;
        Ok(shard_data)
    }

    pub fn save(&self, shard_data: &ShardData) -> Result<(), MetadataError> {
        if let Some(parent) = self.shard_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let data = serde_json::to_string_pretty(shard_data)?;
        fs::write(&self.shard_path, data)?;
        Ok(())
    }

    ///Operations
    pub fn upsert(&self, metadata: Metadata) -> Result<(), MetadataError> {
        let mut shard = self.load()?;

        if let Some(existing) = shard.entries.iter_mut().find(|e| e.file_path == metadata.file_path) {
            *existing = metadata;
        } else {
            shard.entries.push(metadata);
        }

        self.save(&shard)
    }

    pub fn get(&self, canonical_path: &Path) -> Result<Option<Metadata>, MetadataError> {
        let shard = self.load()?;
        Ok(shard.entries.into_iter().find(|e| e.file_path == canonical_path))
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