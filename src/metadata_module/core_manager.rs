use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use crate::metadata_module::error::MetadataError;

///This manages the core metadata folder
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalIndex {
    pub map: HashMap<PathBuf, PathBuf>,
}
pub struct CoreMetadataManager {
    root: PathBuf,
    index_path: PathBuf,
    shards_dir: PathBuf,
}

impl CoreMetadataManager {
    pub fn new(root: PathBuf) -> Self {
        let index_path = root.join("index.json");
        let shards_dir = root.join("shards");

        Self { root, index_path, shards_dir }
    }

    pub fn canonicalize(&self, path: &Path) -> Result<PathBuf, MetadataError> {
        Ok(fs::canonicalize(path)?)
    }

    //Index loading/saving
    pub fn load_index(&self) -> Result<GlobalIndex, MetadataError> {
        if !self.index_path.exists() {
            return Ok(GlobalIndex::default());
        }

        let content = fs::read_to_string(&self.index_path).map_err(|e| MetadataError::CorruptIndex(format!("Failed to read index file: {}", e)))?;
        let index = serde_json::from_str(&content).map_err(|e| MetadataError::CorruptIndex(format!("Failed to parse index file: {}", e)))?;
        Ok(index)
    }

    pub fn save_index(&self, index: &GlobalIndex) -> Result<(), MetadataError> {
        if let Some(parent) = self.index_path.parent(){
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(index)?;
        fs::write(&self.index_path, content)?;
        Ok(())
    }

    //Shard Resolution
    pub fn resolve_shard(&self, canonical_path: &Path) -> PathBuf {
        let dir_name = canonical_path
            .parent()
            .and_then(|p| p.components().next())
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or_else(|| "root".to_string());

        self.shards_dir.join(format!("dir_{}.json", dir_name))
    }

    pub fn lookup_shard(&self, canonical_path: &Path) -> Result<Option<PathBuf>, MetadataError> {
        let index = self.load_index()?;
        Ok(index.map.get(canonical_path).cloned())
    }

    //Index Updating
    pub fn update_index (
        &self, canonical_path: PathBuf,
        shard_path: PathBuf,
    ) -> Result<(), MetadataError> {
        let mut index = self.load_index()?;
        index.map.insert(canonical_path, shard_path);
        self.save_index(&index).map_err(|e| MetadataError::CorruptIndex(format!("Failed to update index: {}", e)))
    }
}