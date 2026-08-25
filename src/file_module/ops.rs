use std::fs;
use std::path::{Path, PathBuf};

use super::manager::FileManager;
use crate::file_module::error::FileManagerError;
use crate::file_validation::handlers::{sanitize_file_name, valid_directory};

impl<'a> FileManager<'a> {
    pub(crate) fn canonical_destination_file(
        src: &Path,
        dst: &Path,
    ) -> Result<PathBuf, FileManagerError> {
        if dst.is_dir() {
            let file_name = src
                .file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid source file name".into()))?;

            // Sanitize file name to prevent path traversal
            let sanitized_name = sanitize_file_name(file_name);

            Ok(dst.join(Path::new(&sanitized_name)))
        } else {
            Ok(dst.to_path_buf())
        }
    }

    pub(crate) fn collect_file_paths(&self, root: &Path) -> Result<Vec<PathBuf>, FileManagerError> {
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

    /// Move a file or directory to the destination.
    pub fn move_path(&self) -> Result<(), FileManagerError> {
        let _guard = self.acquire_lock();
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

        let source_metadata_keys = self.metadata_keys_for_path(src)?;
        let dest_path = Self::canonical_destination_file(src, dst)?;
        fs::rename(src, &dest_path)?;

        if self.settings.enable_metadata {
            let manager = self.metadata_manager_for_destination(&dest_path)?;

            if dest_path.is_file() {
                self.save_metadata_for_file(&dest_path, &manager)?;
            } else if dest_path.is_dir() {
                self.save_metadata_for_directory(&dest_path, &manager)?;
            }

            self.remove_metadata_by_keys(&source_metadata_keys)?;
        }

        if self.settings.verbose {
            println!("Moved {:?} to {:?}", src, dst);
        }

        Ok(())
    }

    /// Delete a file or directory.
    pub fn delete_path(
        &self,
        path: impl Into<PathBuf>,
        recursive: bool,
        to_trash: bool,
    ) -> Result<(), FileManagerError> {
        let _guard = self.acquire_lock();
        let src = path.into();
        let src_path = src.as_path();

        if src_path.is_dir() {
            valid_directory(src_path)?;
        }

        let metadata_keys = if self.settings.dry_run {
            Vec::new()
        } else {
            self.metadata_keys_for_path(src_path)?
        };

        // Trash handling
        if to_trash && self.settings.enable_trash {
            src_path
                .file_name()
                .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;

            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would move {:?} to trash", src_path);
                }
                return Ok(());
            }

            let dst = FileManager::unique_trash_path(src_path)?;
            fs::rename(src_path, &dst)?;

            if self.settings.verbose {
                println!("Moved {:?} to trash", src_path);
            }

            self.remove_metadata_by_keys(&metadata_keys)?;

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
                    println!(
                        "[DRY-RUN] Would permanently delete directory {:?}",
                        src_path
                    );
                }
                return Ok(());
            }

            fs::remove_dir_all(src_path)?;
        } else {
            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would permanently delete file {:?}", src_path);
                }
                return Ok(());
            }

            fs::remove_file(src_path)?;
        }

        self.remove_metadata_by_keys(&metadata_keys)?;

        if self.settings.verbose {
            println!("Permanently deleted {:?}", src_path);
        }

        Ok(())
    }
}
