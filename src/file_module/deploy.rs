use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};

use crate::file_module::compress::CompressionMethod;
use crate::file_module::copy::copy_dir_recursive;
use crate::file_module::error::FileManagerError;
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

pub fn save_manifest(
    source: &Path,
    backup: &Path,
    kind: BackupKind,
    compression_method: Option<CompressionMethod>,
) -> Result<PathBuf, FileManagerError> {
    let original_path = source.to_path_buf();
    let backup_path = fs::canonicalize(backup)?;
    let manifest = DeploymentManifest {
        version: 1,
        kind,
        original_path,
        backup_path,
        compression_method,
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

    let target = destination
        .map(Path::to_path_buf)
        .unwrap_or(manifest.original_path);

    if target.exists() && !force {
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
            if force && target.exists() {
                remove_existing(&target)?;
            }
            restore_copy(backup, &target)?
        }
        BackupKind::Compress => restore_archive(
            backup,
            &target,
            manifest.compression_method.ok_or_else(|| {
                FileManagerError::InvalidInput("Compression method missing from metadata".into())
            })?,
        )?,
    }

    if settings.verbose {
        println!("Deployed {:?} to {:?}", backup, target);
    }
    Ok(target)
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

fn restore_archive(
    backup: &Path,
    target: &Path,
    method: CompressionMethod,
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
        // opened and extracted successfully.
        if target.exists() {
            remove_existing(target)?;
        }
        fs::rename(root, target)?;
        Ok(())
    })();

    let _ = fs::remove_dir_all(&staging);
    result
}

fn remove_existing(path: &Path) -> Result<(), FileManagerError> {
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
    use super::{BackupKind, deploy, save_manifest};
    use crate::settings::Settings;
    use std::fs;

    #[test]
    fn copied_backup_can_be_deployed_to_its_original_path() {
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

        let restored = deploy(&backup, None, false, &Settings::default()).unwrap();

        assert_eq!(restored, original);
        assert_eq!(fs::read_to_string(&original).unwrap(), "deploy me");
        fs::remove_dir_all(root).unwrap();
    }
}
