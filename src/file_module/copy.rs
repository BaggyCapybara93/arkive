use std::fs;
use std::path::Path;

use indicatif::ProgressBar;

use crate::file_module::add_timestamp_to_path;
use crate::file_module::error::FileManagerError;
use crate::file_module::ignore::{IgnoreMatcher, IgnoreStats};
use crate::file_module::manager::FileManager;
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

    let entries: Vec<_> = fs::read_dir(src)?.collect::<Result<Vec<_>, _>>()?;

    for entry in entries.into_iter() {
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
            copy_dir_recursive_filtered(&src_path, &dst_path, progress, matcher, stats)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src_path, &dst_path)?;
        }

        if let Some(bar) = progress {
            bar.inc(1);
        }
    }

    Ok(())
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
            if self.settings.enable_metadata && !self.settings.dry_run {
                if let Ok(manager) = self.metadata_manager_for_destination(&dest_file) {
                    if let Ok(Some(_existing)) = manager.find_metadata(&dest_file) {
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
                }
            }

            fs::copy(src, &dest_file)?;
            validate_hash(src, &dest_file)?;

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
}
