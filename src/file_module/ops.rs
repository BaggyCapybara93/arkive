use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::manager::FileManager;
use crate::file_module::error::FileManagerError;
use crate::file_validation::handlers::{sanitize_file_name, valid_directory};

static TEMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temporary_sibling_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().unwrap_or_else(|| OsStr::new("output"));
    let counter = TEMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut temporary_name = OsString::from(".arkive-tmp-");
    temporary_name.push(std::process::id().to_string());
    temporary_name.push("-");
    temporary_name.push(counter.to_string());
    temporary_name.push("-");
    temporary_name.push(file_name);
    parent.join(temporary_name)
}

pub(crate) fn create_temp_file_sibling(path: &Path) -> Result<PathBuf, FileManagerError> {
    for _ in 0..1000 {
        let candidate = temporary_sibling_path(path);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(FileManagerError::InvalidInput(
        "Could not allocate a temporary sibling path".into(),
    ))
}

pub(crate) fn create_temp_dir_sibling(path: &Path) -> Result<PathBuf, FileManagerError> {
    for _ in 0..1000 {
        let candidate = temporary_sibling_path(path);
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(FileManagerError::InvalidInput(
        "Could not allocate a temporary sibling path".into(),
    ))
}

/// Restore a destination displaced during staged replacement. Preserve both
/// failures, and the displaced path, if the restore itself cannot complete.
pub(crate) fn restore_displaced_after_failed_install(
    displaced: &Path,
    destination: &Path,
    operation: &'static str,
    original: std::io::Error,
) -> FileManagerError {
    match fs::rename(displaced, destination) {
        Ok(()) => FileManagerError::Io(original),
        Err(rollback) => FileManagerError::RollbackFailed {
            operation,
            path: destination.to_path_buf(),
            displaced: displaced.to_path_buf(),
            original,
            rollback,
        },
    }
}

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
    pub fn move_path(&self) -> Result<PathBuf, FileManagerError> {
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
            return Ok(dst.to_path_buf());
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

        Ok(dest_path)
    }

    /// Delete a file or directory.
    pub fn delete_path(
        &self,
        path: impl Into<PathBuf>,
        recursive: bool,
        to_trash: bool,
    ) -> Result<(), FileManagerError> {
        let src = path.into();
        let src_path = src.as_path();
        let trash_path = if to_trash && self.settings.enable_trash {
            Some(Self::trash_path()?)
        } else {
            None
        };
        let _guard = if let Some(trash) = trash_path.as_deref() {
            Self::acquire_paths([src_path, trash])
        } else {
            Self::acquire_paths([src_path])
        };

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

#[cfg(test)]
mod tests {
    use super::{FileManager, restore_displaced_after_failed_install};
    use crate::file_module::FileManagerError;
    use crate::settings::Settings;
    use crate::test::TestDir;
    use std::error::Error;
    use std::io::{self, ErrorKind};

    #[test]
    fn successful_restore_returns_the_original_install_error() {
        let temp = TestDir::new("restore-displaced");
        let displaced = temp.path().join("displaced");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&displaced).unwrap();
        std::fs::write(displaced.join("keep.txt"), b"original").unwrap();

        let error = restore_displaced_after_failed_install(
            &displaced,
            &destination,
            "install staged directory",
            io::Error::new(ErrorKind::NotFound, "staged directory missing"),
        );

        assert!(
            matches!(error, FileManagerError::Io(source) if source.kind() == ErrorKind::NotFound)
        );
        assert_eq!(
            std::fs::read(destination.join("keep.txt")).unwrap(),
            b"original"
        );
        assert!(!displaced.exists());
    }

    #[test]
    fn failed_restore_reports_both_errors_and_displaced_path() {
        let temp = TestDir::new("restore-displaced-failure");
        let displaced = temp.path().join("displaced");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&displaced).unwrap();
        std::fs::write(displaced.join("keep.txt"), b"original").unwrap();
        std::fs::write(&destination, b"obstruction").unwrap();

        let error = restore_displaced_after_failed_install(
            &displaced,
            &destination,
            "install staged directory",
            io::Error::new(ErrorKind::NotFound, "staged directory missing"),
        );

        assert!(matches!(
            &error,
            FileManagerError::RollbackFailed {
                operation: "install staged directory",
                path,
                displaced: saved,
                original,
                rollback: _,
            } if path == &destination && saved == &displaced &&
                original.kind() == ErrorKind::NotFound
        ));
        let message = error.to_string();
        assert!(message.contains("staged directory missing"));
        assert!(message.contains("rollback also failed"));
        assert!(message.contains(&displaced.display().to_string()));
        assert!(
            Error::source(&error)
                .unwrap()
                .downcast_ref::<io::Error>()
                .is_some()
        );
        assert_eq!(
            std::fs::read(displaced.join("keep.txt")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read(destination).unwrap(), b"obstruction");
    }

    #[test]
    fn move_file_removes_source_and_preserves_contents() {
        let temp = TestDir::new("move-file");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");
        std::fs::write(&source, b"move me").unwrap();

        let settings = Settings::default();
        let moved_path = FileManager::new(&source, &destination, &settings)
            .move_path()
            .unwrap();

        assert_eq!(moved_path, destination);
        assert!(!source.exists());
        assert_eq!(std::fs::read(destination).unwrap(), b"move me");
    }

    #[test]
    fn dry_run_move_leaves_source_untouched() {
        let temp = TestDir::new("move-dry-run");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");
        std::fs::write(&source, b"stay here").unwrap();
        let settings = Settings {
            dry_run: true,
            ..Settings::default()
        };

        FileManager::new(&source, &destination, &settings)
            .move_path()
            .unwrap();

        assert_eq!(std::fs::read(source).unwrap(), b"stay here");
        assert!(!destination.exists());
    }

    #[test]
    fn dry_run_delete_leaves_file_untouched() {
        let temp = TestDir::new("delete-dry-run");
        let source = temp.path().join("source.txt");
        std::fs::write(&source, b"do not delete").unwrap();
        let settings = Settings {
            dry_run: true,
            ..Settings::default()
        };

        FileManager::new(&source, "", &settings)
            .delete_path(&source, false, false)
            .unwrap();

        assert_eq!(std::fs::read(source).unwrap(), b"do not delete");
    }
}
