use std::fs;
use std::path::Path;

use crate::file_module::FileManager;
use crate::metadata_module::{handler::MetadataHandler, MetadataManager};
use crate::file_module::error::FileManagerError;

impl<'a> FileManager <'a> {
    pub(crate) fn metadata_manager_for_destination(&self, dst: &Path) -> Result<MetadataManager, FileManagerError> {
        let metadata_root = Self::central_metadata_root(dst)?;
        Ok(MetadataManager::new(metadata_root))
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

    pub(crate) fn save_metadata_for_directory(&self, src: &Path, dst: &Path, manager: &MetadataManager) -> Result<(), FileManagerError> {
        if !self.settings.enable_metadata {
            return Ok(());
        }

        for entry in fs::read_dir(dst)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let dst_path = entry.path();

            if file_type.is_dir() {
                let src_path = src.join(entry.file_name());
                self.save_metadata_for_directory(&src_path, &dst_path, manager)?;
            } else if file_type.is_file() {
                self.save_metadata_for_file(&dst_path, manager)?;
            }
        }

        Ok(())
    }

    pub(crate) fn remove_metadata_for_file(&self, path: &Path) -> Result<(), FileManagerError> {
        if !self.settings.enable_metadata || !path.is_file() {
            return Ok(());
        }

        let manager = self.metadata_manager_for_destination(self.file_dest.as_path())?;

        manager.remove_metadata(path)
            .map(|_| ())
            .map_err(|e| FileManagerError::InvalidInput(format!("Metadata error: {e}")))    
    }
}