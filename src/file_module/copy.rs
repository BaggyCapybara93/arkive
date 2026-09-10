use std::fs;
use std::path::Path;

use indicatif::ProgressBar;

use crate::file_module::add_timestamp_to_path;
use crate::file_module::error::FileManagerError;
use crate::file_module::ignore::{IgnoreMatcher, IgnoreStats};
use crate::file_module::manager::FileManager;
use crate::file_module::ops::{create_temp_dir_sibling, create_temp_file_sibling};
use crate::file_validation::handlers::{ensure_not_nested, valid_directory, validate_hash};

/// Count every file and directory entry in a tree, so we can set an accurate
/// total on the progress bar ONCE before copying starts.
fn count_entries(src: &Path) -> Result<u64, FileManagerError> {
    let mut count = 0u64;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        count += 1; // count this entry itself (file or dir)
        if entry.file_type()?.is_dir() {
            count += count_entries(&entry.path())?;
        }
    }
    Ok(count)
}

/// Recursively copy a directory and its contents to the destination.
pub fn copy_dir_recursive(
    src: &Path,
    dst: &Path,
    progress: Option<&ProgressBar>,
) -> Result<(), FileManagerError> {
    copy_dir_recursive_filtered(src, dst, progress, None, &mut IgnoreStats::default())
}

pub fn copy_dir_recursive_filtered(
    src: &Path,
    dst: &Path,
    progress: Option<&ProgressBar>,
    matcher: Option<&IgnoreMatcher>,
    stats: &mut IgnoreStats,
) -> Result<(), FileManagerError> {
    if src.is_dir() {
        valid_directory(src)?;
    }

    ensure_not_nested(src, dst)?;

    let destination_exists = match fs::symlink_metadata(dst) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(FileManagerError::InvalidInput(format!(
                "Destination {:?} is a symlink",
                dst
            )));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(FileManagerError::InvalidInput(format!(
                "Destination {:?} exists and is not a directory",
                dst
            )));
        }
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    // Build the complete result beside the destination. If anything fails,
    // the original destination remains untouched.
    let staging = create_temp_dir_sibling(dst)?;
    let result = (|| {
        if destination_exists {
            let mut existing_stats = IgnoreStats::default();
            copy_directory_contents(dst, &staging, None, None, false, &mut existing_stats)?;
        }

        copy_directory_contents(src, &staging, progress, matcher, true, stats)?;
        install_staged_directory(&staging, dst, destination_exists)
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }

    result
}

fn copy_directory_contents(
    src: &Path,
    dst: &Path,
    progress: Option<&ProgressBar>,
    matcher: Option<&IgnoreMatcher>,
    verify_files: bool,
    stats: &mut IgnoreStats,
) -> Result<(), FileManagerError> {
    let entries: Vec<_> = fs::read_dir(src)?.collect::<Result<Vec<_>, _>>()?;

    for entry in entries {
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let metadata = entry.metadata()?;

        if matcher.is_some_and(|matcher| {
            matcher.is_excluded(&src_path, file_type.is_dir(), metadata.len())
        }) {
            let previous_entries = stats.entries;
            stats.record(&src_path)?;
            if let Some(bar) = progress {
                bar.inc(stats.entries - previous_entries);
            }
            continue;
        }

        if file_type.is_dir() {
            match fs::symlink_metadata(&dst_path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(FileManagerError::InvalidInput(format!(
                        "Destination {:?} is not a directory",
                        dst_path
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&dst_path)?;
                }
                Err(error) => return Err(error.into()),
            }
            copy_directory_contents(&src_path, &dst_path, progress, matcher, verify_files, stats)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if verify_files {
                copy_file_verified(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
        }

        if let Some(bar) = progress {
            bar.inc(1);
        }
    }

    Ok(())
}

fn copy_file_verified(src: &Path, dst: &Path) -> Result<(), FileManagerError> {
    let staging = create_temp_file_sibling(dst)?;
    let result = (|| {
        fs::copy(src, &staging)?;
        validate_hash(src, &staging)?;
        fs::rename(&staging, dst)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }

    result
}

fn install_staged_directory(
    staging: &Path,
    destination: &Path,
    destination_exists: bool,
) -> Result<(), FileManagerError> {
    if !destination_exists {
        fs::rename(staging, destination)?;
        return Ok(());
    }

    // A non-empty directory cannot be replaced by rename on all supported
    // platforms. Temporarily displace it so a failed install can restore it.
    let displaced = create_temp_dir_sibling(destination)?;
    fs::remove_dir(&displaced)?;
    fs::rename(destination, &displaced)?;

    match fs::rename(staging, destination) {
        Ok(()) => {
            let _ = fs::remove_dir_all(displaced);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&displaced, destination);
            Err(error.into())
        }
    }
}

impl<'a> FileManager<'a> {
    /// Copy a file or directory to the destination.
    pub fn copy_path(
        &self,
        recursive: bool,
        add_timestamp: bool,
    ) -> Result<std::path::PathBuf, FileManagerError> {
        self.copy_path_filtered(recursive, add_timestamp, None)
            .map(|(path, _)| path)
    }

    pub fn copy_path_filtered(
        &self,
        recursive: bool,
        add_timestamp: bool,
        matcher: Option<&IgnoreMatcher>,
    ) -> Result<(std::path::PathBuf, IgnoreStats), FileManagerError> {
        let _guard = self.acquire_lock();
        let mut ignore_stats = IgnoreStats::default();
        let src = self.file_path.as_path();
        let dst = self.file_dest.as_path();
        let final_dst = if add_timestamp {
            add_timestamp_to_path(dst)?
        } else {
            dst.to_path_buf()
        };
        let actual_destination = Self::canonical_destination_file(src, &final_dst)?;

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
                return Ok((actual_destination, ignore_stats));
            }

            let dest_dir = actual_destination.clone();

            // Count the ENTIRE tree once, up front, so the bar's total is
            // accurate for the whole operation, not just the top-level folder.
            let total_entries = count_entries(src)?;
            let progress = Some(FileManager::create_progress_bar(
                total_entries.max(1),
                "Copying directory",
            ));

            copy_dir_recursive_filtered(
                src,
                &dest_dir,
                progress.as_ref(),
                matcher,
                &mut ignore_stats,
            )?;

            // Only finish the bar here, once, after the ENTIRE recursive copy
            // has actually completed.
            if let Some(bar) = progress {
                bar.finish_with_message("Directory copy complete");
            }

            if self.settings.enable_metadata {
                let manager = self.metadata_manager_for_destination(&dest_dir)?;
                self.save_metadata_for_directory(&dest_dir, &manager)?;
            }
        } else {
            if self.settings.dry_run {
                if self.settings.verbose {
                    println!("[DRY-RUN] Would copy file {:?} to {:?}", src, dst);
                }
                return Ok((actual_destination, ignore_stats));
            }

            let dest_file = actual_destination.clone();

            // Check if file already exists in metadata (skip if duplicate)
            if self.settings.enable_metadata
                && !self.settings.dry_run
                && let Ok(manager) = self.metadata_manager_for_destination(&dest_file)
                && let Ok(Some(_existing)) = manager.find_metadata(&dest_file)
            {
                if self.settings.verbose {
                    println!(
                        "File {:?} already exists in metadata, skipping copy",
                        dest_file
                    );
                }

                // Save updated metadata
                self.save_metadata_for_file(&dest_file, &manager)?;

                if self.settings.verbose {
                    println!("Copied {:?} to {:?}", src, dst);
                }

                return Ok((dest_file, ignore_stats));
            }

            copy_file_verified(src, &dest_file)?;

            if self.settings.enable_metadata {
                let manager = self.metadata_manager_for_destination(&dest_file)?;
                self.save_metadata_for_file(&dest_file, &manager)?;
            }
        }

        if self.settings.verbose {
            println!("Copied {:?} to {:?}", src, dst);
        }

        Ok((actual_destination, ignore_stats))
    }
}

#[cfg(test)]
mod tests {
    use super::FileManager;
    use crate::settings::Settings;
    use crate::test::TestDir;

    #[test]
    fn copy_file_preserves_contents() {
        let temp = TestDir::new("copy-file");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");
        std::fs::write(&source, b"arkive test data").unwrap();

        let settings = Settings::default();
        let manager = FileManager::new(&source, &destination, &settings);
        let copied_path = manager.copy_path(false, false).unwrap();

        assert_eq!(copied_path, destination);
        assert_eq!(std::fs::read(destination).unwrap(), b"arkive test data");
        assert_eq!(std::fs::read(source).unwrap(), b"arkive test data");
    }

    #[test]
    fn recursive_copy_preserves_nested_tree() {
        let temp = TestDir::new("copy-tree");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::write(source.join("root.txt"), b"root").unwrap();
        std::fs::write(source.join("nested/child.txt"), b"child").unwrap();

        let settings = Settings::default();
        FileManager::new(&source, &destination, &settings)
            .copy_path(true, false)
            .unwrap();

        assert_eq!(
            std::fs::read(destination.join("root.txt")).unwrap(),
            b"root"
        );
        assert_eq!(
            std::fs::read(destination.join("nested/child.txt")).unwrap(),
            b"child"
        );
    }

    #[test]
    fn dry_run_copy_does_not_create_destination() {
        let temp = TestDir::new("copy-dry-run");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");
        std::fs::write(&source, b"keep me").unwrap();
        let settings = Settings {
            dry_run: true,
            ..Settings::default()
        };

        FileManager::new(&source, &destination, &settings)
            .copy_path(false, false)
            .unwrap();

        assert!(!destination.exists());
        assert_eq!(std::fs::read(source).unwrap(), b"keep me");
    }

    #[test]
    fn failed_file_copy_preserves_existing_destination() {
        let temp = TestDir::new("copy-rollback");
        let source = temp.path().join("missing.txt");
        let destination = temp.path().join("destination.txt");
        std::fs::write(&destination, b"keep me").unwrap();

        let settings = Settings::default();
        let result = FileManager::new(&source, &destination, &settings).copy_path(false, false);

        assert!(result.is_err());
        assert_eq!(std::fs::read(destination).unwrap(), b"keep me");
    }

    #[test]
    fn recursive_copy_preserves_existing_destination_on_success() {
        let temp = TestDir::new("copy-merge");
        let source = temp.path().join("source");
        let destination_parent = temp.path().join("destination");
        let destination = destination_parent.join("source");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(source.join("replace.txt"), b"new").unwrap();
        std::fs::write(source.join("nested/child.txt"), b"child").unwrap();
        std::fs::write(destination.join("keep.txt"), b"keep").unwrap();
        std::fs::write(destination.join("replace.txt"), b"old").unwrap();

        let settings = Settings::default();
        FileManager::new(&source, &destination_parent, &settings)
            .copy_path(true, false)
            .unwrap();

        assert_eq!(
            std::fs::read(destination.join("keep.txt")).unwrap(),
            b"keep"
        );
        assert_eq!(
            std::fs::read(destination.join("replace.txt")).unwrap(),
            b"new"
        );
        assert_eq!(
            std::fs::read(destination.join("nested/child.txt")).unwrap(),
            b"child"
        );
    }

    #[test]
    fn failed_recursive_copy_preserves_existing_destination() {
        let temp = TestDir::new("copy-directory-rollback");
        let source = temp.path().join("source");
        let destination_parent = temp.path().join("destination");
        let destination = destination_parent.join("source");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(source.join("nested/child.txt"), b"new").unwrap();
        // The source directory conflicts with a file in the existing target.
        std::fs::write(destination.join("nested"), b"keep me").unwrap();

        let settings = Settings::default();
        let result =
            FileManager::new(&source, &destination_parent, &settings).copy_path(true, false);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(destination.join("nested")).unwrap(),
            b"keep me"
        );
        assert!(!destination.join("nested/child.txt").exists());
    }
}
