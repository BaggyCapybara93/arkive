use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::file_module::FileManager;
use crate::file_module::copy::copy_dir_recursive;
use crate::file_module::error::FileManagerError;
use crate::file_module::ops::{create_temp_dir_sibling, create_temp_file_sibling, move_no_replace};
use crate::settings::Settings;

fn verify_file_copy(source: &Path, copied: &Path) -> Result<(), FileManagerError> {
    let mut source = File::open(source)?;
    let mut copied = File::open(copied)?;
    let mut source_buffer = [0u8; 64 * 1024];
    let mut copied_buffer = [0u8; 64 * 1024];

    loop {
        let count = source.read(&mut source_buffer)?;
        if count == 0 {
            return if copied.read(&mut copied_buffer[..1])? == 0 {
                Ok(())
            } else {
                Err(FileManagerError::HashMismatch)
            };
        }
        copied.read_exact(&mut copied_buffer[..count])?;
        if source_buffer[..count] != copied_buffer[..count] {
            return Err(FileManagerError::HashMismatch);
        }
    }
}

fn verify_tree_copy(source: &Path, copied: &Path) -> Result<(), FileManagerError> {
    let source_type = fs::symlink_metadata(source)?.file_type();
    let copied_type = fs::symlink_metadata(copied)?.file_type();
    if source_type.is_symlink() || copied_type.is_symlink() {
        return Err(FileManagerError::InvalidInput(format!(
            "Cannot copy a symlink to trash: {source:?}"
        )));
    }
    if source_type.is_file() && copied_type.is_file() {
        return verify_file_copy(source, copied);
    }
    if !source_type.is_dir() || !copied_type.is_dir() {
        return Err(FileManagerError::InvalidInput(format!(
            "Trash copy changed type: {source:?}"
        )));
    }

    let names =
        |directory: &Path| -> Result<std::collections::BTreeSet<OsString>, FileManagerError> {
            fs::read_dir(directory)?
                .map(|entry| {
                    entry
                        .map(|entry| entry.file_name())
                        .map_err(FileManagerError::from)
                })
                .collect()
        };
    let source_names = names(source)?;
    if source_names != names(copied)? {
        return Err(FileManagerError::HashMismatch);
    }
    for name in source_names {
        verify_tree_copy(&source.join(&name), &copied.join(name))?;
    }
    Ok(())
}

fn sync_tree(path: &Path) -> Result<(), FileManagerError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            sync_tree(&entry?.path())?;
        }
        #[cfg(unix)]
        File::open(path)?.sync_all()?;
    } else if metadata.is_file() {
        File::open(path)?.sync_all()?;
    } else {
        return Err(FileManagerError::InvalidInput(format!(
            "Unsupported trash copy entry: {path:?}"
        )));
    }
    Ok(())
}

fn copy_then_move_to_trash(source: &Path, destination: &Path) -> Result<(), FileManagerError> {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(FileManagerError::InvalidInput(format!(
            "Cannot copy a link or special file across filesystems: {source:?}"
        )));
    }
    let staged = if metadata.is_dir() {
        create_temp_dir_sibling(destination)?
    } else {
        create_temp_file_sibling(destination)?
    };
    let mut published = false;
    let result = (|| {
        if metadata.is_dir() {
            copy_dir_recursive(source, &staged, None)?;
        } else {
            fs::copy(source, &staged)?;
        }
        verify_tree_copy(source, &staged)?;
        sync_tree(&staged)?;
        move_no_replace(&staged, destination)?;
        published = true;
        #[cfg(unix)]
        File::open(destination.parent().ok_or_else(|| {
            FileManagerError::InvalidInput(format!(
                "Trash destination has no parent: {destination:?}"
            ))
        })?)?
        .sync_all()?;
        if metadata.is_dir() {
            fs::remove_dir_all(source)?;
        } else {
            fs::remove_file(source)?;
        }
        Ok(())
    })();
    if result.is_err() && !published {
        if metadata.is_dir() {
            let _ = fs::remove_dir_all(&staged);
        } else {
            let _ = fs::remove_file(&staged);
        }
    }
    result
}

fn move_to_trash_with(
    source: &Path,
    destination: &Path,
    attempt: impl FnOnce(&Path, &Path) -> Result<(), FileManagerError>,
) -> Result<(), FileManagerError> {
    match attempt(source, destination) {
        Err(FileManagerError::Io(error)) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_then_move_to_trash(source, destination)
        }
        result => result,
    }
}

pub(crate) fn move_to_trash(source: &Path, destination: &Path) -> Result<(), FileManagerError> {
    move_to_trash_with(source, destination, move_no_replace)
}

impl<'a> FileManager<'a> {
    pub(crate) fn trash_path() -> Result<PathBuf, FileManagerError> {
        Ok(if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join("arkive_trash")
        } else {
            std::env::current_dir()?.join("arkive_trash")
        })
    }

    pub fn trash_dir() -> Result<PathBuf, FileManagerError> {
        let trash = Self::trash_path()?;

        if !trash.exists() {
            std::fs::create_dir_all(&trash)?;
        }

        Ok(trash)
    }

    /// Build a destination in the shared trash directory without replacing an
    /// item that is already there. This is used by every operation that moves
    /// an item to trash, so `list-trash` and `empty-trash` see the same files.
    pub(crate) fn unique_trash_path(src: &Path) -> Result<PathBuf, FileManagerError> {
        let file_name = src
            .file_name()
            .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;
        let trash = Self::trash_dir()?;
        Self::unique_path(&trash.join(file_name))
    }

    fn unique_path(path: &Path) -> Result<PathBuf, FileManagerError> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(path.to_path_buf());
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }

        let parent = path.parent().ok_or_else(|| {
            FileManagerError::InvalidInput(format!("Trash destination has no parent: {:?}", path))
        })?;
        let file_name = path.file_name().ok_or_else(|| {
            FileManagerError::InvalidInput(format!(
                "Trash destination has no file name: {:?}",
                path
            ))
        })?;
        let stem = path.file_stem().unwrap_or(file_name);
        let extension = path.extension();

        for suffix in 1u64.. {
            let mut candidate_name = OsString::from(stem);
            candidate_name.push(format!("-{suffix}"));
            if let Some(extension) = extension {
                candidate_name.push(".");
                candidate_name.push(extension);
            }

            let candidate = parent.join(candidate_name);
            match fs::symlink_metadata(&candidate) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
        }

        unreachable!("u64 suffix space is exhausted")
    }

    pub fn empty_trash(settings: &Settings) -> Result<(), FileManagerError> {
        let lock_path = Self::trash_path()?;
        let _guard = Self::acquire_paths([lock_path.as_path()]);
        let trash = if settings.dry_run {
            let trash = Self::trash_path()?;
            if !trash.exists() {
                if settings.verbose {
                    println!("[DRY-RUN] Trash is already empty");
                }
                return Ok(());
            }
            trash
        } else {
            Self::trash_dir()?
        };

        if !trash.is_dir() {
            return Err(FileManagerError::InvalidDirectory(format!(
                "Trash path is not a directory: {:?}",
                trash
            )));
        }

        // List or remove contents. Dry-run must not create a trash directory
        // or change any of its entries.
        for entry in fs::read_dir(&trash)? {
            let entry = entry?;
            let path = entry.path();

            // Reject symlinks
            let meta = fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                return Err(FileManagerError::PermissionDenied(format!(
                    "Symlink found in trash: {:?}",
                    path
                )));
            }

            if settings.dry_run {
                if settings.verbose {
                    println!("[DRY-RUN] Would permanently delete {:?}", path);
                }
            } else if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }

        if settings.verbose {
            if settings.dry_run {
                println!("[DRY-RUN] Trash would be emptied");
            } else {
                println!("Trash emptied");
            }
        }

        Ok(())
    }

    pub fn list_trash(_settings: &Settings) -> Result<(), FileManagerError> {
        let lock_path = Self::trash_path()?;
        let _guard = Self::acquire_paths([lock_path.as_path()]);
        let trash = Self::trash_dir()?;

        for entry in std::fs::read_dir(trash)? {
            let entry = entry?;
            println!("{:?}", entry.path());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{move_to_trash, move_to_trash_with};
    use crate::file_module::{FileManager, FileManagerError};
    use crate::test::TestDir;
    use std::fs;
    use std::io::{self, ErrorKind};
    use std::path::Path;

    fn cross_device(_: &Path, _: &Path) -> Result<(), FileManagerError> {
        Err(io::Error::from(ErrorKind::CrossesDevices).into())
    }

    #[test]
    fn same_device_trash_move_removes_source() {
        let temp = TestDir::new("trash-same-device");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("trash.txt");
        fs::write(&source, b"save data").unwrap();

        move_to_trash(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read(destination).unwrap(), b"save data");
    }

    #[test]
    fn cross_device_file_is_verified_before_source_removal() {
        let temp = TestDir::new("trash-cross-device-file");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("trash.txt");
        fs::write(&source, b"save data").unwrap();

        move_to_trash_with(&source, &destination, cross_device).unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read(destination).unwrap(), b"save data");
    }

    #[test]
    fn cross_device_directory_keeps_nested_files_and_empty_directories() {
        let temp = TestDir::new("trash-cross-device-directory");
        let source = temp.path().join("source");
        let destination = temp.path().join("trash");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("nested")).unwrap();
        fs::create_dir(source.join("empty")).unwrap();
        fs::write(source.join("nested/save.dat"), b"save data").unwrap();

        move_to_trash_with(&source, &destination, cross_device).unwrap();

        assert!(!source.exists());
        assert_eq!(
            fs::read(destination.join("nested/save.dat")).unwrap(),
            b"save data"
        );
        assert!(destination.join("empty").is_dir());
    }

    #[test]
    fn cross_device_collision_preserves_source_and_existing_trash_item() {
        let temp = TestDir::new("trash-cross-device-collision");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("trash.txt");
        fs::write(&source, b"source").unwrap();
        fs::write(&destination, b"keep").unwrap();

        move_to_trash_with(&source, &destination, cross_device).unwrap_err();

        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(destination).unwrap(), b"keep");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn failed_cross_device_directory_copy_preserves_source() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("trash-cross-device-failed-copy");
        let source = temp.path().join("source");
        let destination = temp.path().join("trash");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("keep.dat"), b"source").unwrap();
        symlink(temp.path().join("missing"), source.join("link")).unwrap();

        move_to_trash_with(&source, &destination, cross_device).unwrap_err();

        assert_eq!(fs::read(source.join("keep.dat")).unwrap(), b"source");
        assert!(
            fs::symlink_metadata(source.join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_in_trash_reserves_its_name() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("trash-dangling-symlink");
        let destination = temp.path().join("save.dat");
        symlink(temp.path().join("missing"), &destination).unwrap();

        assert_eq!(
            FileManager::unique_path(&destination).unwrap(),
            temp.path().join("save-1.dat")
        );
        assert!(
            fs::symlink_metadata(destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
