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
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            count += count_files_recursive(&path)?;
        } else if file_type.is_file() {
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
            let file_type = entry.file_type()?;

            if file_type.is_symlink() {
                continue;
            }

            if file_type.is_dir() {
                // Recurse into subdirectories instead of silently skipping them.
                self.dedup_dir(&path, seen, to_trash, progress)?;
                continue;
            }

            if file_type.is_file() {
                // Convert path to &str safely
                let path_str = path.to_str().ok_or_else(|| {
                    FileManagerError::InvalidInput("Invalid UTF-8 file path".into())
                })?;

                // Compute hash
                let hash = hash_file(path_str)?;

                if let Some(original) = seen.get(&hash) {
                    // Duplicate → delete it
                    self.delete_path(path.clone(), true, to_trash)?;
                    if self.settings.dry_run {
                        println!(
                            "Would remove duplicate: {:?} (original: {:?})",
                            path, original
                        );
                    } else {
                        println!("Removed duplicate: {:?} (original: {:?})", path, original);
                    }
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

        crate::file_validation::handlers::valid_directory(src)?;

        let total_files = count_files_recursive(src)?;
        let progress =
            FileManager::maybe_create_progress_bar(total_files.max(1), "Scanning for duplicates");

        let mut seen: HashMap<String, String> = HashMap::new();
        self.dedup_dir(src, &mut seen, to_trash, progress.as_ref())?;

        if let Some(bar) = progress {
            bar.finish_with_message("Duplicate scan complete");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::FileManager;
    use crate::settings::Settings;
    use crate::test::TestDir;

    #[cfg(unix)]
    #[test]
    fn deduplication_skips_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("dedup-symlink");
        let source = temp.path().join("source");
        let external = temp.path().join("external");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&external).unwrap();
        std::fs::write(source.join("original.txt"), b"same").unwrap();
        std::fs::write(external.join("outside.txt"), b"same").unwrap();
        symlink(&external, source.join("linked-external")).unwrap();

        let settings = Settings::default();
        FileManager::new(&source, "", &settings)
            .folder_deduplication(false)
            .unwrap();

        assert_eq!(
            std::fs::read(external.join("outside.txt")).unwrap(),
            b"same"
        );
    }
}
