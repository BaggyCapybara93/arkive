use std::collections::HashMap;
use std::fs;
use std::path::Path;

use indicatif::ProgressBar;

use crate::file_module::error::FileManagerError;
use crate::file_module::manager::FileManager;
use crate::file_validation::hash::hash_file;

fn count_files_recursive(dir: &Path) -> Result<u64, FileManagerError> {
    let mut count = 0u64;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            count += count_files_recursive(&path)?;
        } else if path.is_file() {
            count += 1;
        }
    }
    Ok(count)
}

impl<'a> FileManager<'a> {
    
    fn dedup_dir(
        &self,
        dir: &std::path::Path,
        seen: &mut HashMap<String, String>,
        to_trash: bool,
        progress: Option<&ProgressBar>,
    ) -> Result<(), FileManagerError> {
        let entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;

        for entry in entries {
            let path = entry.path();

            if path.is_dir() {
                // Recurse into subdirectories instead of silently skipping them.
                self.dedup_dir(&path, seen, to_trash, progress)?;
                continue;
            }

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

                // Only files count toward the total, so only inc here.
                if let Some(bar) = progress {
                    bar.inc(1);
                }
            }
        }

        Ok(())
    }

    /// Scan a directory (recursively) for duplicate files (same hash) and remove them.
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

        let total_files = count_files_recursive(src)?;
        let progress = FileManager::maybe_create_progress_bar(
            total_files.max(1),
            "Scanning for duplicates",
        );

        let mut seen: HashMap<String, String> = HashMap::new();
        self.dedup_dir(src, &mut seen, to_trash, progress.as_ref())?;

        if let Some(bar) = progress {
            bar.finish_with_message("Duplicate scan complete");
        }

        Ok(())
    }
}