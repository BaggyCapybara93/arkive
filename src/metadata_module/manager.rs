use parking_lot::Mutex;
use std::path::Path;

use crate::metadata_module::{
    core_manager::CoreMetadataManager, error::MetadataError, local_manager::LocalMetadataManager,
    structs::Metadata,
};

pub struct MetadataManager {
    core: CoreMetadataManager,
}

// File operations can run concurrently, but the metadata index is a shared
// read-modify-write resource and needs its own short-lived critical section.
static METADATA_LOCK: Mutex<()> = Mutex::new(());

impl MetadataManager {
    pub fn new() -> Result<Self, MetadataError> {
        let core = CoreMetadataManager::new()?;
        Ok(Self { core })
    }

    #[cfg(test)]
    pub(crate) fn from_core(core: CoreMetadataManager) -> Self {
        Self { core }
    }

    pub fn update_metadata(&self, metadata: Metadata) -> Result<(), MetadataError> {
        let _guard = METADATA_LOCK.lock();
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
            return match local.save(&previous_shard) {
                Ok(()) => Err(err),
                Err(rollback) => Err(MetadataError::transaction_rollback(
                    "restore metadata shard after index update failure",
                    shard_path,
                    err,
                    rollback,
                )),
            };
        }

        Ok(())
    }

    pub fn find_metadata(&self, path: &Path) -> Result<Option<Metadata>, MetadataError> {
        let _guard = METADATA_LOCK.lock();
        let canonical = self.core.canonicalize(path)?;

        let shard_path = match self.core.lookup_shard(&canonical)? {
            Some(p) => p,
            None => return Ok(None),
        };

        let local = LocalMetadataManager::new(shard_path);
        local.get(&canonical)
    }

    /// Remove an entry by a canonical path captured before a file operation.
    /// This also works after the file has been moved or deleted.
    pub fn remove_metadata_by_key(&self, canonical_path: &Path) -> Result<bool, MetadataError> {
        let _guard = METADATA_LOCK.lock();
        let canonical = canonical_path.to_path_buf();

        // 1. Look up shard
        let shard_path = match self.core.lookup_shard(&canonical)? {
            Some(p) => p,
            None => return Ok(false),
        };

        // Load the index before changing the shard so an index read failure
        // cannot leave the shard partially updated.
        let mut index = self.core.load_index()?;

        // 2. Remove from shard
        let local = LocalMetadataManager::new(shard_path.clone());
        let previous_shard = local.load()?;
        let removed = local.remove(&canonical)?;

        // 3. Update index if removed
        if removed {
            index.map.remove(&canonical);
            if let Err(err) = self.core.save_index(&index) {
                return match local.save(&previous_shard) {
                    Ok(()) => Err(err),
                    Err(rollback) => Err(MetadataError::transaction_rollback(
                        "restore metadata shard after index removal failure",
                        shard_path,
                        err,
                        rollback,
                    )),
                };
            }
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata_module::{error::MetadataError, handler::MetadataHandler};
    use crate::test::TestDir;
    use std::fs;

    #[test]
    fn failed_index_update_restores_the_previous_shard() {
        let temp = TestDir::new("metadata-update-rollback");
        let source = temp.path().join("save.dat");
        fs::write(&source, b"save data").unwrap();

        let core_dir = temp.path().join("core");
        fs::create_dir_all(core_dir.join("shards")).unwrap();
        fs::create_dir(core_dir.join("index.json")).unwrap();

        let manager = MetadataManager::from_core(CoreMetadataManager::from_core_dir(core_dir));
        let canonical = fs::canonicalize(&source).unwrap();
        let shard_path = manager.core.resolve_shard(&canonical);
        let metadata = MetadataHandler::collect_file(&source).unwrap();

        let error = manager.update_metadata(metadata).unwrap_err();
        assert!(matches!(error, MetadataError::Io { .. }));
        assert!(!shard_path.exists());
    }

    #[test]
    fn failed_index_read_does_not_remove_the_shard() {
        let temp = TestDir::new("metadata-remove-index-read");
        let source = temp.path().join("save.dat");
        fs::write(&source, b"save data").unwrap();

        let core_dir = temp.path().join("core");
        let manager =
            MetadataManager::from_core(CoreMetadataManager::from_core_dir(core_dir.clone()));
        manager
            .update_metadata(MetadataHandler::collect_file(&source).unwrap())
            .unwrap();

        let canonical = fs::canonicalize(&source).unwrap();
        let shard_path = manager.core.resolve_shard(&canonical);
        let index_path = core_dir.join("index.json");
        fs::remove_file(&index_path).unwrap();
        fs::create_dir(&index_path).unwrap();

        let error = manager.remove_metadata_by_key(&canonical).unwrap_err();
        assert!(matches!(error, MetadataError::Io { .. }));
        assert!(shard_path.exists());
    }
}
