use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::metadata_module::error::MetadataError;

///This manages the core metadata folder
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalIndex {
    pub map: HashMap<PathBuf, PathBuf>,
}
pub struct CoreMetadataManager {
    index_path: PathBuf,
    shards_dir: PathBuf,
}

impl CoreMetadataManager {
    pub fn new() -> Result<Self, MetadataError> {
        let exe_dir = std::env::current_exe()
            .map_err(|e| MetadataError::PathError(format!("Failed to get executable path: {}", e)))?
            .parent()
            .ok_or_else(|| MetadataError::PathError("Executable has no parent directory".to_string()))?
            .to_path_buf();
        
        let core_dir = exe_dir.join("core");
        let index_path = core_dir.join("index.json");
        let shards_dir = core_dir.join("shards");

        Ok(Self { index_path, shards_dir })
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
        Self::atomic_write_file(&self.index_path, content.as_bytes())?;
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
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let file_name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "metadata.tmp".to_string());
        let temp_name = format!(".{file_name}.tmp.{timestamp}");
        Ok(path.parent().unwrap_or_else(|| Path::new(".")).join(temp_name))
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