use std::fs;
use std::path::{Path, PathBuf};

use crate::file_validation::handlers::{
    valid_directory
};
use crate::metadata_module::{handler::MetadataHandler, MetadataManager};
use crate::file_module::error::FileManagerError;

use super::manager::FileManager;

impl<'a> FileManager<'a> {
    pub(crate) fn canonical_destination_file(src: &Path, dst: &Path) -> Result<PathBuf, FileManagerError> {
        if dst.is_dir() {
            let file_name = src.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid source file name".into()))?;
            Ok(dst.join(file_name))
        } else {
            Ok(dst.to_path_buf())
        }
    }

    pub(crate) fn central_metadata_file(root: &Path) -> Result<PathBuf, FileManagerError> {
        let root_dir = if root.is_dir() {
            root.to_path_buf()
        } else {
            root.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| Path::new(".").to_path_buf())
        };

        Ok(root_dir.join(".arkive_metadata"))
    }

    pub(crate) fn metadata_manager_for_destination(&self, dst: &Path) -> Result<MetadataManager, FileManagerError> {
        let metadata_path = Self::central_metadata_file(dst)?;
        Ok(MetadataManager::new(metadata_path))
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

    fn collect_file_paths(&self, root: &Path) -> Result<Vec<PathBuf>, FileManagerError> {
        let mut paths = Vec::new();

        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = entry.path();

            if file_type.is_dir() {
                paths.extend(self.collect_file_paths(&path)?);
            } else if file_type.is_file() {
                paths.push(path);
            }
        }

        Ok(paths)
    }

    fn remove_metadata_for_file(&self, path: &Path) -> Result<(), FileManagerError> {
        if !self.settings.enable_metadata || !path.is_file() {
            return Ok(());
        }

        let manager = self.metadata_manager_for_destination(path)?;
        manager.remove_metadata(path)
            .map(|_| ())
            .map_err(|e| FileManagerError::InvalidInput(format!("Metadata error: {e}")))
    }

    /// Move a file or directory to the destination.
    pub fn move_path(&self) -> Result<(), FileManagerError> {
        let _ = self.lock.lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        if src.is_dir() {
            valid_directory(src)?;
        }

        let source_paths = if self.settings.enable_metadata && src.is_dir() {
            Some(self.collect_file_paths(src)?)
        } else {
            None
        };

        if self.settings.dry_run {
            if self.settings.verbose {
                println!("[DRY-RUN] Would move {:?} to {:?}", src, dst);
            }
            return Ok(());
        }

        let dest_path = Self::canonical_destination_file(src, dst)?;
        fs::rename(src, &dest_path)?;

        if self.settings.enable_metadata {
            if dest_path.is_file() {
                let manager = self.metadata_manager_for_destination(&dest_path)?;
                self.save_metadata_for_file(&dest_path, &manager)?;
            } else if dest_path.is_dir() {
                let manager = self.metadata_manager_for_destination(&dest_path)?;
                self.save_metadata_for_directory(src, &dest_path, &manager)?;
            }

            if let Some(paths) = source_paths {
                for old_path in paths {
                    self.remove_metadata_for_file(&old_path)?;
                }
            } else if src.is_file() {
                self.remove_metadata_for_file(src)?;
            }
        }

        if self.settings.verbose {
            println!("Moved {:?} to {:?}", src, dst);
        }

        Ok(())
    }

    /// Delete a file or directory.
    pub fn delete_path(&self, path: impl Into<PathBuf>, recursive: bool, to_trash: bool) -> Result<(), FileManagerError> {
        let _ = self.lock.lock();
        let src = path.into();
        let src_path = src.as_path();

        if src_path.is_dir() {
            valid_directory(src_path)?;
        }

        if to_trash && self.settings.enable_trash {
            let trash = super::trash::trash_dir()?;

            // Extract filename
            let file_name = src_path.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;

            // Build destination inside trash
            let dst = trash.join(file_name);

            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would move {:?} to trash", src_path);
                }
                return Ok(());
            }

            // Move instead of delete
            fs::rename(src_path, dst)?;
            if self.settings.verbose {
                println!("Moved {:?} to trash", src_path);
            }
            return Ok(());
        }

        // Normal delete
        if src_path.is_dir() {
            if !recursive {
                return Err(FileManagerError::InvalidInput(format!(
                    "Use --recursive to delete directories: {:?}",
                    src_path
                )));
            }

            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would permanently delete directory {:?}", src_path);
                }
                return Ok(());
            }

            fs::remove_dir_all(src_path)?;
            if self.settings.verbose {
                println!("Permanently deleted directory {:?}", src_path);
            }
        } else {
            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would permanently delete file {:?}", src_path);
                }
                return Ok(());
            }

            fs::remove_file(src_path)?;
            if self.settings.verbose {
                println!("Permanently deleted file {:?}", src_path);
            }
        }

        Ok(())
    }
}
