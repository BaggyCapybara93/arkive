use std::fs;
use std::path::PathBuf;

use crate::file_validation::handlers::{
    valid_directory, validate_compress_path, validate_hash
};
use crate::file_manager::error::FileManagerError;

use super::manager::FileManager;

impl FileManager {
    /// Move a file or directory to the destination.
    pub fn move_path(&self) -> Result<(), FileManagerError> {
        let _ = self.lock.lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        if src.is_dir() {
            valid_directory(src)?;
        }

        // rename works for both files and directories
        fs::rename(src, dst)?;

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
            super::copy::copy_dir_recursive(src, dst)?;
        } else {
            fs::copy(src, dst)?;
        }

        // Verify file integrity
        if src.is_file() {
            validate_hash(src, dst)?;
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

        if to_trash {
            let trash = Self::trash_dir()?;

            // Extract filename
            let file_name = src_path.file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;

            // Build destination inside trash
            let dst = trash.join(file_name);

            // Move instead of delete
            fs::rename(src_path, dst)?;
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
            fs::remove_dir_all(src_path)?;
        } else {
            fs::remove_file(src_path)?;
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
