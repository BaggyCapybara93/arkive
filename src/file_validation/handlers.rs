use std::path::{Path};
use std::fs::{self, File, OpenOptions};
use crate::file_module::FileManagerError;
use crate::file_validation::hash::hash_file;

pub fn ensure_not_nested(src: &Path, dst: &Path) -> Result<(), FileManagerError>{
    let src = src.canonicalize()?;
    let dst = match dst.canonicalize() {
        Ok(path) => path,
        Err(_) => dst.to_path_buf(),
    };

    if dst.starts_with(&src) {
        return Err(FileManagerError::InvalidInput(
            format!("Destination {:?} cannot be inside source {:?}", dst, src)
        ));
    }

    Ok(())
}

// Validates that the path can be accessed
pub fn validate_access_permissions(path: &Path) -> Result<(), FileManagerError> {
    // Canonicalize
    let canon = path.canonicalize().map_err(|err| {
        match err.kind() {
            std::io::ErrorKind::PermissionDenied =>
                FileManagerError::PermissionDenied(format!("Cannot access {:?}: {err}", path)),
            _ =>
                FileManagerError::InvalidDirectory(format!("Invalid path {:?}: {err}", path)),
        }
    })?;

    // Reject symlinks 
    let meta = fs::symlink_metadata(&canon)?;
    if meta.file_type().is_symlink() {
        return Err(FileManagerError::PermissionDenied(format!(
            "Symlink not allowed: {:?}", canon
        )));
    }

    // Check read permission
    if meta.is_dir() {
        if fs::read_dir(&canon).is_err() {
            return Err(FileManagerError::PermissionDenied(format!(
                "Cannot read directory {:?}", canon
            )));
        }
    } else {
        if File::open(&canon).is_err() {
            return Err(FileManagerError::PermissionDenied(format!(
                "Cannot read file {:?}", canon
            )));
        }
    }

    // Check write permission (only if file exists)
    if meta.is_file() {
        if OpenOptions::new().write(true).open(&canon).is_err() {
            return Err(FileManagerError::PermissionDenied(format!(
                "Cannot write to {:?}", canon
            )));
        }
    }

    // Check delete permission
    if let Some(parent) = canon.parent() {
        if OpenOptions::new().write(true).open(parent).is_err() {
            return Err(FileManagerError::PermissionDenied(format!(
                "Cannot delete {:?} (parent not writable)", canon
            )));
        }
    }

    Ok(())
}


// Validates that the path is a directory and accessible
pub fn valid_directory(path: &Path) -> Result<(), FileManagerError> {
    validate_access_permissions(path)?;

    if !path.exists() {
        return Err(FileManagerError::InvalidDirectory(format!(
            "Directory {:?} does not exist", path
        )));
    }

    if !path.is_dir() {
        return Err(FileManagerError::InvalidDirectory(format!(
            "Path {:?} is not a directory", path
        )));
    }

    Ok(())
}

pub fn validate_hash(src: &Path, dst: &Path) -> Result<(), FileManagerError> {
    let src_hash = hash_file(src.to_str().ok_or(FileManagerError::InvalidInput(
        "Source path is not valid UTF‑8".into(),
    ))?)?;

    let dst_hash = hash_file(dst.to_str().ok_or(FileManagerError::InvalidInput(
        "Destination path is not valid UTF‑8".into(),
    ))?)?;

    if src_hash != dst_hash {
        return Err(FileManagerError::HashMismatch);
    }

    Ok(())
}

// Validates destination is a valid extension for compression
pub fn validate_compress_path(dst: &Path) -> Result<(), FileManagerError> {
    let valid_extensions = ["tar.gz", "tgz"]; // Change this to be configurable in the future
    let dst_str = dst.to_str().ok_or(FileManagerError::InvalidInput(
        "Destination path is not valid UTF‑8".into(),
    ))?;

    if !valid_extensions.iter().any(|ext| dst_str.ends_with(ext)) {
        return Err(FileManagerError::InvalidInput(format!(
            "Destination {:?} must have a valid compression extension: {:?}", dst, valid_extensions
        )));
    }

    Ok(())
}