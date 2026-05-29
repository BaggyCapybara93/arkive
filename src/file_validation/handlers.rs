use std::fs;
use std::path::Path;
use crate::file_manager::FileManagerError;
use crate::file_validation::hash::hash_file;

pub fn ensure_not_nested(src: &Path, dst: &Path) -> Result<(), FileManagerError>{
    let src = src.canonicalize()?;
    let dst = dst.canonicalize().unwrap_or_else(|_| dst.to_path_buf());

    if dst.starts_with(&src) {
        return Err(FileManagerError::InvalidInput(
            format!("Destination {:?} cannot be inside source {:?}", dst, src)
        ));
    }

    Ok(())
}

pub fn valid_directory(path: &Path) -> Result<(), FileManagerError> {
    if !path.exists() {
        return Err(FileManagerError::InvalidInput(format!(
            "Directory {:?} does not exist", path
        )));
    }

    if !path.is_dir() {
        return Err(FileManagerError::InvalidInput(format!(
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