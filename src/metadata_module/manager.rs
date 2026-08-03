use std::path::Path;

use crate::metadata_module::{
    core_manager::CoreMetadataManager,
    local_manager::LocalMetadataManager,
    structs::Metadata,
    error::MetadataError,
};

pub struct MetadataManager {
    core: CoreMetadataManager,
}

impl MetadataManager {
    pub fn new() -> Result<Self, MetadataError> {
        let core = CoreMetadataManager::new()?;
        Ok(Self { core })
    }

    pub fn update_metadata(&self, metadata: Metadata) -> Result<(), MetadataError> {
        let canonical = self.core.canonicalize(&metadata.file_path)?;

        let shard_path = self.core.resolve_shard(&canonical);

        let local = LocalMetadataManager::new(shard_path.clone());
        let previous_shard = local.load()?;
        local.upsert(Metadata {
            file_path: canonical.clone(),
            ..metadata
        })?;

        if let Err(err) = self.core.update_index(canonical.clone(), shard_path.clone()) {
            local.save(&previous_shard)?;
            return Err(err);
        }

        Ok(())
    }

    pub fn find_metadata(&self, path: &Path) -> Result<Option<Metadata>, MetadataError> {
        let canonical = self.core.canonicalize(path)?;

        let shard_path = match self.core.lookup_shard(&canonical)? {
            Some(p) => p,
            None => return Ok(None),
        };

        let local = LocalMetadataManager::new(shard_path);
        Ok(local.get(&canonical)?)
    }

    pub fn remove_metadata(&self, path: &Path) -> Result<bool, MetadataError> {
        let canonical = self.core.canonicalize(path)?;

        // 1. Look up shard
        let shard_path = match self.core.lookup_shard(&canonical)? {
            Some(p) => p,
            None => return Ok(false),
        };

        // 2. Remove from shard
        let local = LocalMetadataManager::new(shard_path.clone());
        let removed = local.remove(&canonical)?;

        // 3. Update index if removed
        if removed {
            let mut index = self.core.load_index()?;
            index.map.remove(&canonical);
            self.core.save_index(&index).map_err(|e| MetadataError::CorruptIndex(format!("Failed to save index after removal: {}", e)))?;
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("arkive-{name}-{unique}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
