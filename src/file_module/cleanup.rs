use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::file_module::FileManager;
use crate::file_module::error::FileManagerError;
use crate::file_validation::handlers::valid_directory;

fn cleanup_io(operation: &'static str, path: &Path, source: std::io::Error) -> FileManagerError {
    FileManagerError::IoAt {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

impl<'a> FileManager<'a> {
    /// Clean up the workspace with multiple options
    ///
    /// Flags:
    /// - empty_trash: Empty the arkive trash directory
    /// - deduplicate: Scan for and remove duplicate files
    /// - scan_unused: Scan for empty or unused files
    /// - scan_empty_dirs: Scan for and remove empty directories
    pub fn cleanup(&self, options: CleanupOptions) -> Result<(), FileManagerError> {
        if options.empty_trash {
            FileManager::empty_trash(self.settings)?;
        }

        if options.deduplicate {
            self.folder_deduplication(true)?;
        }

        if options.scan_unused {
            self.scan_unused_files()?;
        }

        if options.scan_empty_dirs {
            self.scan_and_remove_empty_dirs()?;
        }

        Ok(())
    }

    /// Scan for unused files (files that haven't been accessed recently)
    fn scan_unused_files(&self) -> Result<(), FileManagerError> {
        let src = self.file_path.as_path();

        valid_directory(src)?;

        let now = SystemTime::now();
        let thirty_days = 30 * 24 * 60 * 60;
        let progress = Some(FileManager::create_spinner("Scanning for unused files"));

        // Collect matches instead of just printing them, so the scan
        // actually reports something usable when it's done.
        let mut unused_files: Vec<PathBuf> = Vec::new();

        Self::scan_directory_recursive(
            &src.to_path_buf(),
            now,
            thirty_days,
            self,
            progress.as_ref(),
            &mut unused_files,
        )?;

        if let Some(bar) = progress {
            bar.finish_with_message(format!(
                "Scan complete: {} unused file(s) found",
                unused_files.len()
            ));
        }

        if self.settings.verbose {
            println!("Scan complete: {} unused file(s) found", unused_files.len());
            for path in &unused_files {
                println!("  Unused (not accessed in 30+ days): {:?}", path);
            }
        }

        Ok(())
    }

    fn scan_file(
        path: &PathBuf,
        now: SystemTime,
        thirty_days: u64,
        settings: &FileManager<'_>,
        unused_files: &mut Vec<PathBuf>,
    ) -> Result<(), FileManagerError> {
        if let Ok(metadata) = fs::metadata(path)
            && let Ok(accessed) = metadata.accessed()
        {
            let elapsed = now
                .duration_since(accessed)
                .map_err(|e| FileManagerError::InvalidInput(e.to_string()))?;
            let elapsed_secs = elapsed.as_secs();

            if elapsed_secs > thirty_days {
                if settings.settings.verbose {
                    println!("Found unused file (not accessed in 30+ days): {:?}", path);
                }
                // Actually record the match instead of discarding it.
                unused_files.push(path.clone());
            }
        }
        Ok(())
    }

    fn scan_directory_recursive(
        dir: &PathBuf,
        now: SystemTime,
        thirty_days: u64,
        settings: &FileManager<'_>,
        progress: Option<&indicatif::ProgressBar>,
        unused_files: &mut Vec<PathBuf>,
    ) -> Result<(), FileManagerError> {
        let entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type()?;

            if file_type.is_symlink() {
                continue;
            }

            Self::scan_file(&path, now, thirty_days, settings, unused_files)?;

            if file_type.is_dir() {
                Self::scan_directory_recursive(
                    &path,
                    now,
                    thirty_days,
                    settings,
                    progress,
                    unused_files,
                )?;
            }

            if let Some(bar) = progress {
                bar.inc(1);
            }
        }
        Ok(())
    }

    /// Scan for and remove empty directories
    fn scan_and_remove_empty_dirs(&self) -> Result<(), FileManagerError> {
        let src = self.file_path.as_path();

        valid_directory(src)?;

        let mut empty_dirs: Vec<PathBuf> = Vec::new();
        let progress = Some(FileManager::create_spinner(
            "Scanning for empty directories",
        ));

        Self::find_empty_dirs_recursive(&src.to_path_buf(), &mut empty_dirs, progress.as_ref())?;

        if let Some(bar) = progress {
            bar.finish_with_message("Empty directory scan complete");
        }

        if self.settings.dry_run {
            if self.settings.verbose {
                for dir_path in empty_dirs.iter().rev() {
                    println!("[DRY-RUN] Would remove empty directory: {:?}", dir_path);
                }
                println!(
                    "[DRY-RUN] {} empty directories would be removed",
                    empty_dirs.len()
                );
            }
            return Ok(());
        }

        // Remove empty directories in reverse order to handle nested paths.
        let empty_count =
            Self::remove_empty_dirs(empty_dirs.into_iter().rev(), self.settings.verbose)?;

        if self.settings.verbose {
            println!("Scan complete: {} empty directories removed", empty_count);
        }

        Ok(())
    }

    fn remove_empty_dirs(
        paths: impl IntoIterator<Item = PathBuf>,
        verbose: bool,
    ) -> Result<usize, FileManagerError> {
        let mut removed = 0;
        for path in paths {
            fs::remove_dir(&path)
                .map_err(|error| cleanup_io("remove empty directory", &path, error))?;
            if verbose {
                println!("Removing empty directory: {:?}", path);
            }
            removed += 1;
        }
        Ok(removed)
    }

    fn find_empty_dirs_recursive(
        path: &PathBuf,
        empty_dirs: &mut Vec<PathBuf>,
        progress: Option<&indicatif::ProgressBar>,
    ) -> Result<(), FileManagerError> {
        let entries =
            fs::read_dir(path).map_err(|error| cleanup_io("scan directory", path, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| cleanup_io("read directory entry", path, error))?;
            let entry_path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| cleanup_io("inspect directory entry", &entry_path, error))?;

            if file_type.is_symlink() {
                continue;
            }

            if file_type.is_dir() {
                Self::find_empty_dirs_recursive(&entry_path, empty_dirs, progress)?;

                // Check if directory is empty after children processed.
                // A subdirectory that was JUST found empty is still on
                // disk at this point (actual deletion happens later, in
                // scan_and_remove_empty_dirs), so a plain count would
                // wrongly see it as "not empty" and this parent would
                // never be flagged. Exclude anything already queued
                // for removal from the count.
                let sub_entries = fs::read_dir(&entry_path)
                    .map_err(|error| cleanup_io("scan directory", &entry_path, error))?;
                let sub_entries = sub_entries
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| cleanup_io("read directory entry", &entry_path, error))?;
                let remaining = sub_entries
                    .into_iter()
                    .filter(|e| !empty_dirs.contains(&e.path()))
                    .count();
                if remaining == 0 {
                    empty_dirs.push(entry_path.clone());
                }
            }

            if let Some(bar) = progress {
                bar.inc(1);
            }
        }
        Ok(())
    }
}

/// Options for the cleanup operation
#[derive(Debug, Clone, Default)]
pub struct CleanupOptions {
    /// Empty the arkive trash directory
    pub empty_trash: bool,

    /// Scan for and remove duplicate files
    pub deduplicate: bool,

    /// Scan for unused files (not accessed in 30+ days)
    pub scan_unused: bool,

    /// Scan for and remove empty directories
    pub scan_empty_dirs: bool,
}

#[cfg(test)]
mod tests {
    use super::{CleanupOptions, FileManager};
    use crate::file_module::FileManagerError;
    use crate::settings::Settings;
    use crate::test::TestDir;
    use std::io::ErrorKind;

    #[test]
    fn empty_directory_scan_reports_read_failure_with_path() {
        let temp = TestDir::new("cleanup-scan-error");
        let missing = temp.path().join("missing");
        let error =
            FileManager::find_empty_dirs_recursive(&missing, &mut Vec::new(), None).unwrap_err();

        assert!(matches!(
            error,
            FileManagerError::IoAt {
                operation: "scan directory",
                path,
                source,
            } if path == missing && source.kind() == ErrorKind::NotFound
        ));
    }

    #[test]
    fn empty_directory_removal_reports_failure_with_path() {
        let temp = TestDir::new("cleanup-remove-error");
        let nonempty = temp.path().join("nonempty");
        std::fs::create_dir(&nonempty).unwrap();
        std::fs::write(nonempty.join("keep.txt"), b"keep").unwrap();

        let error = FileManager::remove_empty_dirs(vec![nonempty.clone()], false).unwrap_err();

        assert!(matches!(
            error,
            FileManagerError::IoAt {
                operation: "remove empty directory",
                path,
                ..
            } if path == nonempty
        ));
        assert!(nonempty.join("keep.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_does_not_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("cleanup-symlink");
        let source = temp.path().join("source");
        let external = temp.path().join("external");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&external).unwrap();
        std::fs::write(external.join("outside.txt"), b"keep").unwrap();
        symlink(&external, source.join("linked-external")).unwrap();

        let settings = Settings::default();
        FileManager::new(&source, "", &settings)
            .cleanup(CleanupOptions {
                scan_unused: true,
                scan_empty_dirs: true,
                ..CleanupOptions::default()
            })
            .unwrap();

        assert_eq!(
            std::fs::read(external.join("outside.txt")).unwrap(),
            b"keep"
        );
        assert!(source.join("linked-external").exists());
    }
}
