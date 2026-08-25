use std::fs;
use std::path::Path;

use crate::file_module::FileManager;
use crate::metadata_module::{handler::MetadataHandler, MetadataManager};
use crate::file_module::error::FileManagerError;

impl<'a> FileManager <'a> {
    pub(crate) fn metadata_manager_for_destination(&self, _dst: &Path) -> Result<MetadataManager, FileManagerError> {
        Ok(MetadataManager::new().map_err(|e| FileManagerError::InvalidInput(e.to_string()))?)
    }

    pub(crate) fn save_metadata_for_file(&self, path: &Path, manager: &MetadataManager) -> Result<(), FileManagerError> {
        if !self.settings.enable_metadata || !path.is_file() {
            return Ok(());
        }

        let metadata = MetadataHandler::collect_file(path)
            .map_err(|e| FileManagerError::InvalidInput(format!("Metadata error: {e}")))?;

        manager.update_metadata(metadata)
            .map_err(|e| FileManagerError::InvalidInput(format!("Metadata error: {e}")))?;

        Ok(())
    }

    pub(crate) fn save_metadata_for_directory(&self, dst: &Path, manager: &MetadataManager) -> Result<(), FileManagerError> {
        if !self.settings.enable_metadata {
            return Ok(());
        }

        for entry in fs::read_dir(dst)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let dst_path = entry.path();

            if file_type.is_dir() {
                self.save_metadata_for_directory(&dst_path, manager)?;
            } else if file_type.is_file() {
                self.save_metadata_for_file(&dst_path, manager)?;
            }
        }

        Ok(())
    }

    /// Capture canonical metadata keys before moving or deleting a path.
    pub(crate) fn metadata_keys_for_path(&self, path: &Path) -> Result<Vec<std::path::PathBuf>, FileManagerError> {
        if !self.settings.enable_metadata {
            return Ok(Vec::new());
        }

        let paths = if path.is_dir() {
            self.collect_file_paths(path)?
        } else {
            vec![path.to_path_buf()]
        };

        paths
            .into_iter()
            .map(|path| {
                fs::canonicalize(&path).map_err(|e| {
                    FileManagerError::InvalidInput(format!("Metadata path error for {:?}: {e}", path))
                })
            })
            .collect()
    }

    pub(crate) fn remove_metadata_by_keys(&self, keys: &[std::path::PathBuf]) -> Result<(), FileManagerError> {
        if !self.settings.enable_metadata || keys.is_empty() {
            return Ok(());
        }

        let manager = self.metadata_manager_for_destination(self.file_dest.as_path())?;
        for key in keys {
            manager
                .remove_metadata_by_key(key)
                .map_err(|e| FileManagerError::InvalidInput(format!("Metadata error: {e}")))?;
        }

        Ok(())
    }
}
