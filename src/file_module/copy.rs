use std::fs;
use std::path::Path;

#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(unix)]
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[cfg(unix)]
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, openat, statat};
#[cfg(unix)]
use rustix::io::Errno;
#[cfg(unix)]
use sha2::{Digest, Sha256};

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

#[cfg(not(unix))]
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

        if file_type.is_symlink() {
            return Err(FileManagerError::InvalidInput(format!(
                "Directory copy cannot contain symbolic links: {src_path:?}"
            )));
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

#[cfg(unix)]
fn copy_directory_contents(
    src: &Path,
    dst: &Path,
    progress: Option<&ProgressBar>,
    matcher: Option<&IgnoreMatcher>,
    verify_files: bool,
    stats: &mut IgnoreStats,
) -> Result<(), FileManagerError> {
    let source = openat(
        rustix::fs::CWD,
        src,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    let mut source = Dir::new(source).map_err(std::io::Error::from)?;
    copy_open_directory_contents(
        &mut source,
        src,
        dst,
        progress,
        matcher,
        verify_files,
        stats,
    )
}

#[cfg(unix)]
fn copy_open_directory_contents(
    source: &mut Dir,
    src: &Path,
    dst: &Path,
    progress: Option<&ProgressBar>,
    matcher: Option<&IgnoreMatcher>,
    verify_files: bool,
    stats: &mut IgnoreStats,
) -> Result<(), FileManagerError> {
    while let Some(entry) = source.read() {
        let entry = entry.map_err(std::io::Error::from)?;
        let name = OsStr::from_bytes(entry.file_name().to_bytes());
        if name == "." || name == ".." {
            continue;
        }
        let src_path = src.join(name);
        let dst_path = dst.join(name);
        let metadata = statat(
            source.fd().map_err(std::io::Error::from)?,
            name,
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(std::io::Error::from)?;
        let file_type = FileType::from_raw_mode(metadata.st_mode);

        // Preserve ignore rules for symlinks, including size rules, without
        // ever opening the link as a file to copy.
        let size = if file_type.is_symlink() && matcher.is_some() {
            statat(
                source.fd().map_err(std::io::Error::from)?,
                name,
                AtFlags::empty(),
            )
            .map_err(std::io::Error::from)?
            .st_size
        } else {
            metadata.st_size
        };
        if matcher.is_some_and(|matcher| {
            matcher.is_excluded(&src_path, file_type.is_dir(), size.max(0) as u64)
        }) {
            let previous_entries = stats.entries;
            stats.record(&src_path)?;
            if let Some(bar) = progress {
                bar.inc(stats.entries - previous_entries);
            }
            continue;
        }

        if file_type.is_symlink() {
            return Err(FileManagerError::InvalidInput(format!(
                "Directory copy cannot contain symbolic links: {src_path:?}"
            )));
        }

        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        if file_type.is_dir() {
            let opened = openat(
                source.fd().map_err(std::io::Error::from)?,
                name,
                flags | OFlags::DIRECTORY,
                Mode::empty(),
            )
            .map_err(std::io::Error::from)?;
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
            let mut child = Dir::new(opened).map_err(std::io::Error::from)?;
            copy_open_directory_contents(
                &mut child,
                &src_path,
                &dst_path,
                progress,
                matcher,
                verify_files,
                stats,
            )?;
        } else if file_type.is_file() {
            let opened = openat(
                source.fd().map_err(std::io::Error::from)?,
                name,
                flags | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| {
                if error == Errno::LOOP {
                    FileManagerError::InvalidInput(format!(
                        "Directory copy cannot contain symbolic links: {src_path:?}"
                    ))
                } else {
                    FileManagerError::Io(error.into())
                }
            })?;
            let mut opened = File::from(opened);
            if !opened.metadata()?.is_file() {
                return Err(FileManagerError::InvalidInput(format!(
                    "Directory copy source changed type: {src_path:?}"
                )));
            }
            copy_open_file(&mut opened, &dst_path, verify_files)?;
        } else {
            return Err(FileManagerError::InvalidInput(format!(
                "Directory copy requires regular files and directories: {src_path:?}"
            )));
        }

        if let Some(bar) = progress {
            bar.inc(1);
        }
    }

    Ok(())
}

#[cfg(unix)]
fn copy_open_file(src: &mut File, dst: &Path, verify: bool) -> Result<(), FileManagerError> {
    let target = if verify {
        create_temp_file_sibling(dst)?
    } else {
        dst.to_path_buf()
    };
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&target)?;
        let mut source_hash = Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let read = src.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            if verify {
                source_hash.update(&buffer[..read]);
            }
        }
        drop(output);
        fs::set_permissions(&target, src.metadata()?.permissions())?;

        if verify {
            let mut copied = File::open(&target)?;
            let mut copied_hash = Sha256::new();
            loop {
                let read = copied.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                copied_hash.update(&buffer[..read]);
            }
            if source_hash.finalize() != copied_hash.finalize() {
                return Err(FileManagerError::HashMismatch);
            }
            fs::rename(&target, dst)?;
        }
        Ok(())
    })();

    if result.is_err() && verify {
        let _ = fs::remove_file(&target);
    }
    result
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
    use super::{FileManager, copy_dir_recursive, copy_dir_recursive_filtered};
    use crate::file_module::ignore::{IgnoreMatcher, IgnoreOptions, IgnoreStats};
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

    #[cfg(unix)]
    #[test]
    fn recursive_copy_rejects_nested_source_symlink_without_replacing_destination() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("copy-source-symlink");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        let outside = temp.path().join("secret.txt");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(&outside, b"private data").unwrap();
        std::fs::write(destination.join("keep.txt"), b"keep").unwrap();
        symlink("../../secret.txt", source.join("nested/linked.txt")).unwrap();

        let result = copy_dir_recursive(&source, &destination, None);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(destination.join("keep.txt")).unwrap(),
            b"keep"
        );
        assert!(!destination.join("nested/linked.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn recursive_copy_rejects_existing_destination_symlink_without_materializing_it() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("copy-destination-symlink");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        let outside = temp.path().join("secret.txt");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir_all(destination.join("nested")).unwrap();
        std::fs::write(source.join("new.txt"), b"new").unwrap();
        std::fs::write(&outside, b"private data").unwrap();
        symlink(&outside, destination.join("nested/linked.txt")).unwrap();

        let result = copy_dir_recursive(&source, &destination, None);

        assert!(result.is_err());
        assert!(
            std::fs::symlink_metadata(destination.join("nested/linked.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!destination.join("new.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn recursive_copy_allows_excluded_symlink_without_copying_its_target() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("copy-excluded-symlink");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        let outside = temp.path().join("secret.txt");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("keep.txt"), b"keep").unwrap();
        std::fs::write(&outside, b"private data").unwrap();
        symlink(&outside, source.join("linked.txt")).unwrap();
        let matcher = IgnoreMatcher::build(
            &source,
            &IgnoreOptions {
                no_global: true,
                no_local: true,
                excludes: vec!["linked.txt".into()],
                ..IgnoreOptions::default()
            },
        )
        .unwrap();
        let mut stats = IgnoreStats::default();

        copy_dir_recursive_filtered(&source, &destination, None, Some(&matcher), &mut stats)
            .unwrap();

        assert_eq!(
            std::fs::read(destination.join("keep.txt")).unwrap(),
            b"keep"
        );
        assert!(!destination.join("linked.txt").exists());
        assert_eq!(stats.entries, 1);
    }

    #[cfg(unix)]
    #[test]
    fn open_directory_copy_keeps_the_original_tree_after_path_replacement() {
        use super::copy_open_directory_contents;
        use crate::file_module::ignore::IgnoreStats;
        use rustix::fs::{Dir, Mode, OFlags, openat};
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("copy-replaced-directory");
        let source = temp.path().join("source");
        let moved = temp.path().join("moved");
        let outside = temp.path().join("outside");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(source.join("file.txt"), b"original").unwrap();
        std::fs::write(outside.join("file.txt"), b"private data").unwrap();
        let opened = openat(
            rustix::fs::CWD,
            &source,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap();
        let mut opened = Dir::new(opened).unwrap();
        std::fs::rename(&source, &moved).unwrap();
        symlink(&outside, &source).unwrap();

        copy_open_directory_contents(
            &mut opened,
            &source,
            &destination,
            None,
            None,
            true,
            &mut IgnoreStats::default(),
        )
        .unwrap();

        assert_eq!(
            std::fs::read(destination.join("file.txt")).unwrap(),
            b"original"
        );
    }
}
