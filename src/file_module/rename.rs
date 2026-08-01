use crate::file_module::error::FileManagerError;
use crate::file_module::FileManager;

impl<'a> FileManager<'a> {
    /// Rename a file or directory to the destination.
    pub fn rename_path(&self) -> Result<(), FileManagerError> {
        let _guard = self.acquire_lock();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();

        if src.is_dir() {
            // Note: rename_path doesn't currently support recursive rename
            // but we can use move_path logic if needed.
            // For now, we'll use the standard fs::rename.
        }

        if self.settings.dry_run {
            if self.settings.verbose {
                println!("[DRY-RUN] Would rename {:?} to {:?}", src, dst);
            }
            return Ok(());
        }

        let dest_path = Self::canonical_destination_file(src, dst)?;
        std::fs::rename(src, &dest_path)?;

        if self.settings.enable_metadata {
            let manager = self.metadata_manager_for_destination(&dest_path)?;

            if dest_path.is_file() {
                self.save_metadata_for_file(&dest_path, &manager)?;
            } else if dest_path.is_dir() {
                self.save_metadata_for_directory(src, &dest_path, &manager)?;
            }

            // Remove old metadata
            let source_paths = if self.settings.enable_metadata && src.is_dir() {
                Some(self.collect_file_paths(src)?)
            } else {
                None
            };

            if let Some(paths) = source_paths {
                for old_path in paths {
                    self.remove_metadata_for_file(&old_path)?;
                }
            } else if src.is_file() {
                self.remove_metadata_for_file(src)?;
            }
        }

        Ok(())
    }
}