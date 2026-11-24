use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
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

const MANAGED_DIR: &str = ".arkive-items";

#[derive(Serialize, Deserialize)]
struct TrashMetadata {
    original_path: PathBuf,
    deleted_at: chrono::DateTime<chrono::Utc>,
}

fn store_item(source: &Path, trash: &Path) -> Result<String, FileManagerError> {
    let name = source
        .file_name()
        .ok_or_else(|| FileManagerError::InvalidInput("Invalid file name".into()))?;
    let parent = source
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let original_path = fs::canonicalize(parent)?.join(name);
    let managed = trash.join(MANAGED_DIR);
    fs::create_dir_all(&managed)?;
    let (id, item) = (0u64..)
        .find_map(|n| {
            let id = format!(
                "item-{}-{n}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
            );
            let item = managed.join(&id);
            match fs::create_dir(&item) {
                Ok(()) => Some(Ok((id, item))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .expect("ID space exhausted")?;
    let metadata = TrashMetadata {
        original_path,
        deleted_at: chrono::Utc::now(),
    };
    let result = (|| -> Result<(), FileManagerError> {
        let bytes = serde_json::to_vec(&metadata).map_err(|e| {
            FileManagerError::InvalidInput(format!("Cannot encode trash metadata: {e}"))
        })?;
        let mut file = File::create(item.join("metadata.json"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        move_to_trash(source, &item.join("data"))
    })();
    if let Err(error) = result {
        // A cross-device move can fail after publishing the copy. Keep its
        // metadata and payload in that case so the copy remains recoverable.
        if fs::symlink_metadata(item.join("data")).is_err() {
            let _ = fs::remove_dir_all(&item);
        }
        return Err(error);
    }
    Ok(id)
}

fn read_metadata(item: &Path) -> Result<TrashMetadata, FileManagerError> {
    serde_json::from_reader(File::open(item.join("metadata.json"))?).map_err(|e| {
        FileManagerError::InvalidInput(format!("Invalid trash metadata at {item:?}: {e}"))
    })
}

fn restore_item(
    trash: &Path,
    id: &str,
    destination: Option<&Path>,
    dry_run: bool,
) -> Result<(), FileManagerError> {
    let (source, original, item) = if let Some(name) = id.strip_prefix("legacy:") {
        if name.is_empty()
            || Path::new(name).components().count() != 1
            || name == "."
            || name == ".."
            || name == MANAGED_DIR
        {
            return Err(FileManagerError::InvalidInput("Invalid trash ID".into()));
        }
        (trash.join(name), None, None)
    } else {
        if !id.starts_with("item-")
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-' || b.is_ascii_lowercase())
            || id.contains('/')
        {
            return Err(FileManagerError::InvalidInput("Invalid trash ID".into()));
        }
        let item = trash.join(MANAGED_DIR).join(id);
        let metadata = read_metadata(&item)?;
        (item.join("data"), Some(metadata.original_path), Some(item))
    };
    fs::symlink_metadata(&source)?;
    let target = destination
        .map(Path::to_path_buf)
        .or(original)
        .ok_or_else(|| {
            FileManagerError::InvalidInput("Legacy trash items require --destination".into())
        })?;
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let resolved_parent = fs::canonicalize(parent)?;
    if resolved_parent.starts_with(fs::canonicalize(trash)?) {
        return Err(FileManagerError::InvalidInput(
            "Cannot restore into the trash directory".into(),
        ));
    }
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("Destination already exists: {target:?}"),
            )
            .into());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    if dry_run {
        println!("[DRY-RUN] Would restore {id} to {}", target.display());
    } else {
        move_to_trash(&source, &target)?;
        if let Some(item) = item {
            fs::remove_dir_all(item)?;
        }
        println!("Restored {id} to {}", target.display());
    }
    Ok(())
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

    pub(crate) fn trash_item(src: &Path) -> Result<String, FileManagerError> {
        store_item(src, &Self::trash_dir()?)
    }

    pub fn restore_trash(
        id: &str,
        destination: Option<&Path>,
        settings: &Settings,
    ) -> Result<(), FileManagerError> {
        let trash = Self::trash_path()?;
        let _guard = Self::acquire_paths([trash.as_path()]);
        restore_item(&trash, id, destination, settings.dry_run)
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

        for entry in fs::read_dir(&trash)? {
            let entry = entry?;
            if entry.file_name() == MANAGED_DIR {
                for item in fs::read_dir(entry.path())? {
                    let item = item?;
                    let metadata = read_metadata(&item.path())?;
                    println!(
                        "{}\t{}\t{}",
                        item.file_name().to_string_lossy(),
                        metadata.deleted_at.to_rfc3339(),
                        metadata.original_path.display()
                    );
                }
            } else {
                println!(
                    "legacy:{}\tunknown\tunknown",
                    entry.file_name().to_string_lossy()
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{move_to_trash, move_to_trash_with};
    use crate::file_module::FileManagerError;
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
    #[test]
    fn managed_item_records_metadata_and_restores() {
        let temp = TestDir::new("trash-round-trip");
        let trash = temp.path().join("trash");
        fs::create_dir(&trash).unwrap();
        let source = temp.path().join("save.dat");
        fs::write(&source, b"save").unwrap();
        let before = chrono::Utc::now();
        let id = super::store_item(&source, &trash).unwrap();
        let item = trash.join(super::MANAGED_DIR).join(&id);
        let metadata = super::read_metadata(&item).unwrap();
        assert_eq!(
            metadata.original_path,
            fs::canonicalize(temp.path()).unwrap().join("save.dat")
        );
        assert!(metadata.deleted_at >= before && metadata.deleted_at <= chrono::Utc::now());
        super::restore_item(&trash, &id, None, true).unwrap();
        assert!(!source.exists());
        super::restore_item(&trash, &id, None, false).unwrap();
        assert_eq!(fs::read(source).unwrap(), b"save");
        assert!(!item.exists());
    }

    #[test]
    fn collisions_preserve_both_items_and_alternate_destination_works() {
        let temp = TestDir::new("trash-restore-collision");
        let trash = temp.path().join("trash");
        fs::create_dir(&trash).unwrap();
        let source = temp.path().join("save");
        fs::write(&source, b"trashed").unwrap();
        let id = super::store_item(&source, &trash).unwrap();
        fs::write(&source, b"existing").unwrap();
        assert!(super::restore_item(&trash, &id, None, false).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"existing");
        let alternate = temp.path().join("alternate");
        super::restore_item(&trash, &id, Some(&alternate), false).unwrap();
        assert_eq!(fs::read(alternate).unwrap(), b"trashed");
    }

    #[test]
    fn legacy_requires_destination_and_rejects_invalid_ids() {
        let temp = TestDir::new("trash-legacy");
        let trash = temp.path().join("trash");
        fs::create_dir(&trash).unwrap();
        fs::write(trash.join("old"), b"legacy").unwrap();
        assert!(super::restore_item(&trash, "legacy:old", None, false).is_err());
        assert!(
            super::restore_item(
                &trash,
                "legacy:../old",
                Some(&temp.path().join("bad")),
                false
            )
            .is_err()
        );
        let target = temp.path().join("restored");
        super::restore_item(&trash, "legacy:old", Some(&target), false).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"legacy");
    }
}
