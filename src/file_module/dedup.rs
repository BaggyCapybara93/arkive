use std::collections::HashMap;
use std::fs;

use crate::file_module::error::FileManagerError;
use crate::file_module::manager::FileManager;
use crate::file_validation::hash::hash_file;

impl<'a> FileManager<'a> {
    /// Scan a directory for duplicate files (same hash) and remove them.
    pub fn folder_deduplication(&self, to_trash: bool) -> Result<(), FileManagerError> {
        let src = self.file_path.as_path();

        if src.is_dir() {
            crate::file_validation::handlers::valid_directory(src)?;
        }

        if !src.is_dir() {
            return Err(FileManagerError::InvalidInput(
                "Deduplication requires a directory".into(),
            ));
        }

        let mut seen: HashMap<String, String> = HashMap::new();
        let entries: Vec<_> = fs::read_dir(src)?.collect::<Result<Vec<_>, _>>()?;
        let progress = FileManager::maybe_create_progress_bar(entries.len().max(1) as u64, "Scanning for duplicates");

        for entry in entries {
            let path = entry.path();

            if path.is_file() {
                // Convert path to &str safely
                let path_str = path.to_str().ok_or_else(|| {
                    FileManagerError::InvalidInput("Invalid UTF-8 file path".into())
                })?;

                // Compute hash
                let hash = hash_file(path_str)?;

                if let Some(original) = seen.get(&hash) {
                    // Duplicate → delete it
                    self.delete_path(path.clone(), true, to_trash)?;
                    println!("Removed duplicate: {:?} (original: {:?})", path, original);
                } else {
                    seen.insert(hash, path_str.to_string());
                }
            }

            if let Some(bar) = &progress {
                bar.inc(1);
            }
        }

        if let Some(bar) = progress {
            bar.finish_with_message("Duplicate scan complete");
        }

        Ok(())
    }
}
