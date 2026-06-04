use std::collections::HashMap;
use std::fs;

use crate::file_manager::error::FileManagerError;
use crate::file_manager::manager::FileManager;
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

        for entry in fs::read_dir(src)? {
            let entry = entry?;
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
        }

        Ok(())
    }
}
