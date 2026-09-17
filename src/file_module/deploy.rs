use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};

use crate::file_module::compress::CompressionMethod;
use crate::file_module::copy::copy_dir_recursive;
use crate::file_module::error::FileManagerError;
use crate::file_module::ops::{create_temp_dir_sibling, create_temp_file_sibling};
use crate::settings::Settings;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupKind {
    Copy,
    Move,
    Compress,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeploymentManifest {
    version: u8,
    kind: BackupKind,
    original_path: PathBuf,
    backup_path: PathBuf,
    compression_method: Option<CompressionMethod>,
    #[serde(default)]
    ignore_rules: Vec<String>,
    #[serde(default)]
    partial_move: bool,
    #[serde(with = "chrono::serde::ts_seconds")]
    created_at: DateTime<Utc>,
}

pub fn manifest_path(backup: &Path) -> Result<PathBuf, FileManagerError> {
    let name = backup
        .file_name()
        .ok_or_else(|| FileManagerError::InvalidInput("Backup path has no file name".into()))?;
    let mut manifest_name = name.to_os_string();
    manifest_name.push(".arkive.json");
    Ok(backup.with_file_name(manifest_name))
}

#[cfg(test)]
pub fn save_manifest(
    source: &Path,
    backup: &Path,
    kind: BackupKind,
    compression_method: Option<CompressionMethod>,
) -> Result<PathBuf, FileManagerError> {
    save_manifest_with_ignores(source, backup, kind, compression_method, &[], false)
}

pub fn save_manifest_with_ignores(
    source: &Path,
    backup: &Path,
    kind: BackupKind,
    compression_method: Option<CompressionMethod>,
    ignore_rules: &[String],
    partial_move: bool,
) -> Result<PathBuf, FileManagerError> {
    let original_path = source.to_path_buf();
    let backup_path = fs::canonicalize(backup)?;
    let manifest = DeploymentManifest {
        version: 1,
        kind,
        original_path,
        backup_path,
        compression_method,
        ignore_rules: ignore_rules.to_vec(),
        partial_move,
        created_at: Utc::now(),
    };
    let path = manifest_path(backup)?;
    let data = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| FileManagerError::InvalidInput(error.to_string()))?;
    fs::write(&path, data)?;
    Ok(path)
}

pub fn deploy(
    backup: &Path,
    destination: Option<&Path>,
    force: bool,
    use_recorded_destination: bool,
    settings: &Settings,
) -> Result<PathBuf, FileManagerError> {
    let manifest_file = manifest_path(backup)?;
    let data = fs::read(&manifest_file).map_err(|error| {
        FileManagerError::InvalidInput(format!(
            "Could not read deployment metadata {:?}: {error}",
            manifest_file
        ))
    })?;
    let manifest: DeploymentManifest = serde_json::from_slice(&data).map_err(|error| {
        FileManagerError::InvalidInput(format!("Invalid deployment metadata: {error}"))
    })?;

    if manifest.version != 1 {
        return Err(FileManagerError::InvalidInput(format!(
            "Unsupported deployment metadata version: {}",
            manifest.version
        )));
    }

    let target = match destination {
        Some(destination) => destination.to_path_buf(),
        None if use_recorded_destination => manifest.original_path,
        None => {
            return Err(FileManagerError::InvalidInput(
                "Provide --destination or --use-recorded-destination to authorize the restore target"
                    .into(),
            ));
        }
    };

    let can_merge_partial_move = manifest.partial_move
        && matches!(manifest.kind, BackupKind::Move)
        && target.is_dir()
        && backup.is_dir();

    let target_is_occupied = target_is_occupied(&target)?;

    if target_is_occupied && !force {
        return Err(FileManagerError::InvalidInput(format!(
            "Restore destination {:?} already exists; use --force to replace it",
            target
        )));
    }

    if settings.dry_run {
        println!("[DRY-RUN] Would deploy {:?} to {:?}", backup, target);
        return Ok(target);
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    match manifest.kind {
        BackupKind::Copy | BackupKind::Move => {
            if force && target_is_occupied && can_merge_partial_move {
                // A partial move intentionally merges the backed-up subset into
                // the existing source tree. `copy_dir_recursive` stages the
                // complete merged result before replacing the destination.
                restore_copy(backup, &target)?;
            } else {
                let staged = stage_copy(backup, &target)?;
                let result = publish_staged(&staged, &target, force);
                if result.is_err() {
                    let _ = remove_staged(&staged);
                }
                result?;
            }
        }
        BackupKind::Compress => restore_archive(
            backup,
            &target,
            manifest.compression_method.ok_or_else(|| {
                FileManagerError::InvalidInput("Compression method missing from metadata".into())
            })?,
            force,
        )?,
    }

    if settings.verbose {
        println!("Deployed {:?} to {:?}", backup, target);
    }
    Ok(target)
}

fn target_is_occupied(path: &Path) -> Result<bool, FileManagerError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn restore_copy(backup: &Path, target: &Path) -> Result<(), FileManagerError> {
    if backup.is_dir() {
        copy_dir_recursive(backup, target, None)
    } else if backup.is_file() {
        fs::copy(backup, target)?;
        Ok(())
    } else {
        Err(FileManagerError::InvalidInput(format!(
            "Backup does not exist: {:?}",
            backup
        )))
    }
}

/// Build a complete deployment beside its final destination before touching
/// that destination. This keeps failed reads and copies from damaging a
/// previous restore.
fn stage_copy(backup: &Path, target: &Path) -> Result<PathBuf, FileManagerError> {
    let metadata = fs::symlink_metadata(backup)?;
    if metadata.file_type().is_symlink() {
        return Err(FileManagerError::InvalidInput(format!(
            "Backup must not be a symlink: {backup:?}"
        )));
    }

    if metadata.is_dir() {
        let staged = create_temp_dir_sibling(target)?;
        let result = copy_dir_recursive(backup, &staged, None);
        if result.is_err() {
            let _ = fs::remove_dir_all(&staged);
        }
        result.map(|()| staged)
    } else if metadata.is_file() {
        let staged = create_temp_file_sibling(target)?;
        let result = fs::copy(backup, &staged).map(|_| ());
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result.map_err(Into::into).map(|()| staged)
    } else {
        Err(FileManagerError::InvalidInput(format!(
            "Backup does not exist or is not a regular file/directory: {backup:?}"
        )))
    }
}

/// Publish a staged file or directory. Without `--force`, every creation is
/// no-clobber, including a destination that appears after the initial CLI
/// validation. With `--force`, keep the displaced entry until the staged
/// replacement is installed so a failed final rename can be rolled back.
fn publish_staged(staged: &Path, target: &Path, force: bool) -> Result<(), FileManagerError> {
    if target_is_occupied(target)? {
        if !force {
            return Err(FileManagerError::InvalidInput(format!(
                "Restore destination {target:?} appeared while preparing deployment; refusing to overwrite it"
            )));
        }
        return replace_staged(staged, target);
    }

    if force {
        // `--force` explicitly authorizes replacing a target that appears
        // between this check and the final rename.
        return match fs::rename(staged, target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                replace_staged(staged, target)
            }
            Err(error) => Err(error.into()),
        };
    }

    publish_new(staged, target)
}

fn publish_new(staged: &Path, target: &Path) -> Result<(), FileManagerError> {
    let metadata = fs::symlink_metadata(staged)?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(FileManagerError::InvalidInput(format!(
            "Staged deployment is not a regular file or directory: {staged:?}"
        )));
    }

    if metadata.is_file() {
        install_file_no_clobber(staged, target)
    } else {
        // `create_dir` is an exclusive reservation: a late creator cannot be
        // overwritten by the directory deployment.
        fs::create_dir(target)?;
        let result = move_directory_contents_no_clobber(staged, target);
        if result.is_ok() {
            fs::remove_dir(staged)?;
        } else {
            // This directory was exclusively created by this operation, so it
            // is safe to clean up a partial deployment on failure.
            let _ = fs::remove_dir_all(target);
        }
        result
    }
}

fn install_file_no_clobber(staged: &Path, target: &Path) -> Result<(), FileManagerError> {
    match fs::hard_link(staged, target) {
        Ok(()) => {
            fs::remove_file(staged)?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Err(FileManagerError::InvalidInput(format!(
                "Restore destination {target:?} appeared while preparing deployment; refusing to overwrite it"
            )))
        }
        // SMB/NFS and some Windows filesystems do not permit hard links. An
        // exclusive create retains the no-clobber guarantee on those shares.
        Err(_) => copy_file_no_clobber(staged, target),
    }
}

fn copy_file_no_clobber(source: &Path, target: &Path) -> Result<(), FileManagerError> {
    let mut output = match OpenOptions::new().write(true).create_new(true).open(target) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(FileManagerError::InvalidInput(format!(
                "Restore destination {target:?} appeared while preparing deployment; refusing to overwrite it"
            )));
        }
        Err(error) => return Err(error.into()),
    };
    let result: Result<(), io::Error> = (|| {
        let mut input = fs::File::open(source)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        Ok(())
    })();
    drop(output);
    if result.is_ok() {
        fs::remove_file(source)?;
    } else {
        let _ = fs::remove_file(target);
    }
    result.map_err(Into::into)
}

fn move_directory_contents_no_clobber(
    source: &Path,
    target: &Path,
) -> Result<(), FileManagerError> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(FileManagerError::InvalidInput(format!(
                "Staged deployment contains an unsupported entry: {source_path:?}"
            )));
        }
        if metadata.is_dir() {
            fs::create_dir(&target_path)?;
            move_directory_contents_no_clobber(&source_path, &target_path)?;
            fs::remove_dir(&source_path)?;
        } else {
            install_file_no_clobber(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn replace_staged(staged: &Path, target: &Path) -> Result<(), FileManagerError> {
    let displaced = create_temp_dir_sibling(target)?;
    fs::remove_dir(&displaced)?;
    fs::rename(target, &displaced)?;

    match fs::rename(staged, target) {
        Ok(()) => {
            let _ = remove_staged(&displaced);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&displaced, target);
            Err(error.into())
        }
    }
}

fn restore_archive(
    backup: &Path,
    target: &Path,
    method: CompressionMethod,
    force: bool,
) -> Result<(), FileManagerError> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let staging = parent.join(format!(
        ".arkive-deploy-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir(&staging)?;

    let result = (|| {
        let file = fs::File::open(backup)?;
        match method {
            CompressionMethod::Gzip => tar::Archive::new(GzDecoder::new(file)).unpack(&staging)?,
            CompressionMethod::Zstd => {
                let decoder = zstd::Decoder::new(file)?;
                tar::Archive::new(decoder).unpack(&staging)?;
            }
        }

        let mut entries = fs::read_dir(&staging)?;
        let root = entries
            .next()
            .transpose()?
            .ok_or_else(|| FileManagerError::InvalidInput("Archive is empty".into()))?
            .path();
        if entries.next().is_some() {
            return Err(FileManagerError::InvalidInput(
                "Archive metadata expected exactly one top-level item".into(),
            ));
        }
        // Do not replace an existing deployment until the archive has been
        // opened and extracted successfully. `publish_staged` also refuses a
        // target that appeared during extraction unless --force was supplied.
        publish_staged(&root, target, force)
    })();

    let _ = fs::remove_dir_all(&staging);
    result
}

fn remove_staged(path: &Path) -> Result<(), FileManagerError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BackupKind, deploy, publish_new, publish_staged, save_manifest, save_manifest_with_ignores,
    };
    use crate::settings::Settings;
    use crate::test::TestDir;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[test]
    fn copied_backup_requires_explicit_authorization_for_recorded_destination() {
        let root = std::env::temp_dir().join(format!(
            "arkive-deploy-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let original = root.join("original.txt");
        let backup = root.join("backup.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&original, "deploy me").unwrap();
        fs::copy(&original, &backup).unwrap();
        save_manifest(&original, &backup, BackupKind::Copy, None).unwrap();
        fs::remove_file(&original).unwrap();

        let denied = deploy(&backup, None, false, false, &Settings::default());
        assert!(denied.is_err());
        assert!(!original.exists());

        let explicit_target = root.join("explicit-target.txt");
        let explicitly_restored = deploy(
            &backup,
            Some(&explicit_target),
            false,
            false,
            &Settings::default(),
        )
        .unwrap();
        assert_eq!(explicitly_restored, explicit_target);
        assert_eq!(fs::read_to_string(&explicit_target).unwrap(), "deploy me");

        let restored = deploy(&backup, None, false, true, &Settings::default()).unwrap();

        assert_eq!(restored, original);
        assert_eq!(fs::read_to_string(&original).unwrap(), "deploy me");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_move_cannot_merge_into_existing_destination_without_force() {
        let root = std::env::temp_dir().join(format!(
            "arkive-deploy-partial-move-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let target = root.join("target");
        let backup = root.join("backup");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::write(target.join("replace.txt"), "original").unwrap();
        fs::write(backup.join("replace.txt"), "backup").unwrap();
        save_manifest_with_ignores(&target, &backup, BackupKind::Move, None, &[], true).unwrap();

        let denied = deploy(&backup, Some(&target), false, false, &Settings::default());

        assert!(denied.is_err());
        assert_eq!(
            fs::read_to_string(target.join("replace.txt")).unwrap(),
            "original"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_backup_deploys_through_the_staged_no_clobber_path() {
        let temp = TestDir::new("deploy-directory");
        let original = temp.path().join("original");
        let backup = temp.path().join("backup");
        let target = temp.path().join("target");
        fs::create_dir_all(backup.join("nested")).unwrap();
        fs::write(backup.join("save.dat"), b"save").unwrap();
        fs::write(backup.join("nested/settings.dat"), b"settings").unwrap();
        save_manifest(&original, &backup, BackupKind::Copy, None).unwrap();

        deploy(&backup, Some(&target), false, false, &Settings::default()).unwrap();

        assert_eq!(fs::read(target.join("save.dat")).unwrap(), b"save");
        assert_eq!(
            fs::read(target.join("nested/settings.dat")).unwrap(),
            b"settings"
        );
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_destination_requires_force() {
        let root = std::env::temp_dir().join(format!(
            "arkive-deploy-dangling-link-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let backup = root.join("backup.txt");
        let target = root.join("target.txt");
        let external_target = root.join("external.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&backup, "backup").unwrap();
        symlink(&external_target, &target).unwrap();
        save_manifest(&target, &backup, BackupKind::Copy, None).unwrap();

        let denied = deploy(&backup, None, false, true, &Settings::default());

        assert!(denied.is_err());
        assert!(!external_target.exists());
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn late_file_destination_is_not_overwritten_without_force() {
        let temp = TestDir::new("deploy-late-file");
        let staged = temp.path().join("staged.txt");
        let target = temp.path().join("target.txt");
        fs::write(&staged, b"deployment").unwrap();
        fs::write(&target, b"late writer").unwrap();

        // Model a destination created after `deploy` made its initial check.
        assert!(publish_new(&staged, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"late writer");
        assert_eq!(fs::read(&staged).unwrap(), b"deployment");
    }

    #[test]
    fn late_directory_destination_is_not_overwritten_without_force() {
        let temp = TestDir::new("deploy-late-directory");
        let staged = temp.path().join("staged");
        let target = temp.path().join("target");
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("replacement.txt"), b"deployment").unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep.txt"), b"late writer").unwrap();

        // This is the directory equivalent of a late destination collision.
        assert!(publish_new(&staged, &target).is_err());
        assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"late writer");
        assert_eq!(
            fs::read(staged.join("replacement.txt")).unwrap(),
            b"deployment"
        );
    }

    #[test]
    fn forced_publish_replaces_only_after_staging_is_complete() {
        let temp = TestDir::new("deploy-force-replace");
        let staged = temp.path().join("staged");
        let target = temp.path().join("target");
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("replacement.txt"), b"deployment").unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep.txt"), b"previous deployment").unwrap();

        publish_staged(&staged, &target, true).unwrap();
        assert!(!staged.exists());
        assert_eq!(
            fs::read(target.join("replacement.txt")).unwrap(),
            b"deployment"
        );
        assert!(!target.join("keep.txt").exists());
    }
}
