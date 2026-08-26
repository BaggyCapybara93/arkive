use std::path::Path;

use crate::metadata_module::{
    core_manager::CoreMetadataManager, error::MetadataError, local_manager::LocalMetadataManager,
    structs::Metadata,
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

        if let Err(err) = self
            .core
            .update_index(canonical.clone(), shard_path.clone())
        {
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

    /// Remove an entry by a canonical path captured before a file operation.
    /// This also works after the file has been moved or deleted.
    pub fn remove_metadata_by_key(&self, canonical_path: &Path) -> Result<bool, MetadataError> {
        let canonical = canonical_path.to_path_buf();

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
            self.core.save_index(&index).map_err(|e| {
                MetadataError::CorruptIndex(format!("Failed to save index after removal: {}", e))
            })?;
        }

        Ok(removed)
    }
}
