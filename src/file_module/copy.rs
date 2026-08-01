use std::fs;
use std::path::Path;

use crate::file_module::error::FileManagerError;
use crate::file_module::manager::FileManager;
use crate::file_validation::handlers::{ensure_not_nested, valid_directory, validate_hash};

/// Recursively copy a directory and its contents to the destination.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), FileManagerError> {
    if src.is_dir() {
        valid_directory(src)?;
    }

    ensure_not_nested(src, dst)?;

    if dst.exists() {
        if !dst.is_dir() {
            return Err(FileManagerError::InvalidInput(format!(
                "Destination {:?} exists and is not a directory",
                dst
            )));
        }
    } else {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

impl<'a> FileManager<'a> {
    /// Copy a file or directory to the destination.
    pub fn copy_path(&self, recursive: bool) -> Result<(), FileManagerError> {
        let _guard = self.acquire_lock();
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
            copy_dir_recursive(src, &dest_dir)?;

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

            // Check if file already exists in metadata (skip if duplicate)
            if self.settings.enable_metadata && !self.settings.dry_run {
                if let Ok(manager) = self.metadata_manager_for_destination(&dest_file) {
                    if let Ok(Some(_existing)) = manager.find_metadata(&dest_file) {
                        if self.settings.verbose {
                            println!("File {:?} already exists in metadata, skipping copy", dest_file);
                        }

                        // Save updated metadata
                        self.save_metadata_for_file(&dest_file, &manager)?;

                        if self.settings.verbose {
                            println!("Copied {:?} to {:?}", src, dst);
                        }

                        return Ok(());
                    }
                }
            }

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
}