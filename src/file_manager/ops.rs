use std::fs;
use std::path::{Path, PathBuf};

use crate::file_validation::handlers::{
    valid_directory, validate_compress_path, validate_hash
};
use crate::metadata_manager::{handler::MetadataHandler, MetadataManager};
use crate::file_manager::error::FileManagerError;

use super::manager::FileManager;

impl<'a> FileManager<'a> {
    fn canonical_destination_file(src: &Path, dst: &Path) -> Result<PathBuf, FileManagerError> {
        if dst.is_dir() {
            let file_name = src.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid source file name".into()))?;
            Ok(dst.join(file_name))
        } else {
            Ok(dst.to_path_buf())
        }
    }

    fn central_metadata_file(root: &Path) -> Result<PathBuf, FileManagerError> {
        let root_dir = if root.is_dir() {
            root.to_path_buf()
        } else {
            root.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| Path::new(".").to_path_buf())
        };

        Ok(root_dir.join(".arkive_metadata.json"))
    }

    fn metadata_manager_for_destination(&self, dst: &Path) -> Result<MetadataManager, FileManagerError> {
        let metadata_path = Self::central_metadata_file(dst)?;
        Ok(MetadataManager::new(metadata_path))
    }

    fn save_metadata_for_file(&self, path: &Path, manager: &MetadataManager) -> Result<(), FileManagerError> {
        if !self.settings.enable_metadata || !path.is_file() {
            return Ok(());
        }

        let metadata = MetadataHandler::collect_file(path)
            .map_err(|e| FileManagerError::InvalidInput(format!("Metadata error: {e}")))?;

        manager.update_metadata(metadata)
            .map_err(|e| FileManagerError::InvalidInput(format!("Metadata error: {e}")))?;

        Ok(())
    }

    fn save_metadata_for_directory(&self, src: &Path, dst: &Path, manager: &MetadataManager) -> Result<(), FileManagerError> {
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

    /// Move a file or directory to the destination.
    pub fn move_path(&self) -> Result<(), FileManagerError> {
        let _ = self.lock.lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        if src.is_dir() {
            valid_directory(src)?;
        }

        if self.settings.dry_run {
            if self.settings.verbose {
                println!("[DRY-RUN] Would move {:?} to {:?}", src, dst);
            }
            return Ok(());
        }

        let dest_path = Self::canonical_destination_file(src, dst)?;
        fs::rename(src, &dest_path)?;

        if self.settings.enable_metadata && dest_path.is_file() {
            let manager = self.metadata_manager_for_destination(&dest_path)?;
            self.save_metadata_for_file(&dest_path, &manager)?;
        }

        if self.settings.verbose {
            println!("Moved {:?} to {:?}", src, dst);
        }

        Ok(())
    }

    /// Copy a file or directory to the destination.
    pub fn copy_path(&self, recursive: bool) -> Result<(), FileManagerError> {
        let _ = self.lock.lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        if src.is_dir() {
            valid_directory(src)?;
            if !recursive {
                return Err(FileManagerError::InvalidInput(format!(
                    "Use --recursive to copy directories: {:?}",
                    src
                )));
            }

            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would copy directory {:?} to {:?}", src, dst);
                }
                return Ok(());
            }

            let dest_dir = Self::canonical_destination_file(src, dst)?;
            super::copy::copy_dir_recursive(src, &dest_dir)?;

            if self.settings.enable_metadata {
                let manager = self.metadata_manager_for_destination(&dest_dir)?;
                self.save_metadata_for_directory(src, &dest_dir, &manager)?;
            }
        } else {
            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would copy file {:?} to {:?}", src, dst);
                }
                return Ok(());
            }

            let dest_file = Self::canonical_destination_file(src, dst)?;
            fs::copy(src, &dest_file)?;

            if self.settings.enable_metadata {
                let manager = self.metadata_manager_for_destination(&dest_file)?;
                self.save_metadata_for_file(&dest_file, &manager)?;
            }
        }

        // Verify file integrity
        if src.is_file() && !self.settings.dry_run {
            let dest_file = Self::canonical_destination_file(src, dst)?;
            validate_hash(src, &dest_file)?;
        }

        if self.settings.verbose {
            println!("Copied {:?} to {:?}", src, dst);
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

    /// Compress a file or directory into a tar.gz archive.
    pub fn compress_path(&self) -> Result<(), FileManagerError> {
        let _ = self.lock.lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        // Ensure destination is valid for compression
        validate_compress_path(dst)?;

        if src.is_dir() {
            valid_directory(src)?;
        }
        
        let tar_gz = fs::File::create(dst)?;
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);

        if src.is_dir() {
            let src_name = src.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid directory name".into()))?;
            tar.append_dir_all(src_name, src)?;
        } else {
            let name = src.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;
            tar.append_path_with_name(src, name)?;
        }

        tar.finish()?;
        Ok(())
    }
}
