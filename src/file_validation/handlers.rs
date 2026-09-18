use crate::file_module::FileManagerError;
use crate::file_module::compress::CompressionMethod;
use crate::file_validation::hash::hash_file;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::path::Path;

pub fn sanitize_file_name(file_name: &OsStr) -> String {
    // Check for path traversal attempts
    if file_name.to_string_lossy().contains("..") {
        return String::new();
    }

    // Get the file name as a string
    let name = file_name.to_string_lossy();

    // Return empty if the name is empty or contains only path separators
    if name.is_empty() || name == "/" || name == "\\" {
        return String::new();
    }

    name.to_string()
}

pub fn ensure_not_nested(src: &Path, dst: &Path) -> Result<(), FileManagerError> {
    let src = src.canonicalize()?;
    match fs::symlink_metadata(dst) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(FileManagerError::InvalidInput(format!(
                "Destination {:?} is a symlink",
                dst
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let dst = normalize_path_for_comparison(dst)?;

    if dst.starts_with(&src) {
        return Err(FileManagerError::InvalidInput(format!(
            "Destination {:?} cannot be inside source {:?}",
            dst, src
        )));
    }

    Ok(())
}

/// Normalize a path even when its final components do not exist yet. The
/// existing prefix is canonicalized so relative destinations can still be
/// compared with the canonical source path.
fn normalize_path_for_comparison(path: &Path) -> Result<std::path::PathBuf, FileManagerError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    if let Ok(canonical) = absolute.canonicalize() {
        return Ok(canonical);
    }

    let mut existing = absolute.clone();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().ok_or_else(|| {
                    FileManagerError::InvalidInput(format!(
                        "Cannot normalize destination {:?}",
                        path
                    ))
                })?;
                missing.push(name.to_os_string());
                existing.pop();
            }
            Err(error) => return Err(error.into()),
        }
    }

    let mut normalized = existing.canonicalize()?;
    for component in missing.iter().rev() {
        normalized.push(component);
    }
    Ok(normalized)
}

// Validates that the path can be accessed
pub fn validate_access_permissions(path: &Path) -> Result<(), FileManagerError> {
    // Inspect the caller-supplied path before canonicalizing it. Canonicalizing
    // first would resolve a symlink and make the later symlink check observe
    // the target instead of the link.
    let input_meta = fs::symlink_metadata(path).map_err(|err| match err.kind() {
        std::io::ErrorKind::PermissionDenied => {
            FileManagerError::PermissionDenied(format!("Cannot access {:?}: {err}", path))
        }
        _ => FileManagerError::InvalidDirectory(format!("Invalid path {:?}: {err}", path)),
    })?;
    if input_meta.file_type().is_symlink() {
        return Err(FileManagerError::PermissionDenied(format!(
            "Symlink not allowed: {:?}",
            path
        )));
    }

    let canon = path.canonicalize().map_err(|err| match err.kind() {
        std::io::ErrorKind::PermissionDenied => {
            FileManagerError::PermissionDenied(format!("Cannot access {:?}: {err}", path))
        }
        _ => FileManagerError::InvalidDirectory(format!("Invalid path {:?}: {err}", path)),
    })?;

    let meta = fs::metadata(&canon)?;

    // Check read permission
    if meta.is_dir() {
        if fs::read_dir(&canon).is_err() {
            return Err(FileManagerError::PermissionDenied(format!(
                "Cannot read directory {:?}",
                canon
            )));
        }
    } else {
        if File::open(&canon).is_err() {
            return Err(FileManagerError::PermissionDenied(format!(
                "Cannot read file {:?}",
                canon
            )));
        }
    }

    // Check write permission (only if file exists)
    if meta.is_file() && OpenOptions::new().write(true).open(&canon).is_err() {
        return Err(FileManagerError::PermissionDenied(format!(
            "Cannot write to {:?}",
            canon
        )));
    }

    // Check delete permission (only for files, not directories)
    // This check ensures the parent directory is writable so we can delete the file
    if meta.is_file()
        && let Some(parent) = canon.parent()
        && OpenOptions::new().write(true).open(parent).is_err()
    {
        return Err(FileManagerError::PermissionDenied(format!(
            "Cannot delete {:?} (parent not writable)",
            canon
        )));
    }

    Ok(())
}

// Validates that the path is a directory and accessible
pub fn valid_directory(path: &Path) -> Result<(), FileManagerError> {
    validate_access_permissions(path)?;

    if !path.exists() {
        return Err(FileManagerError::InvalidDirectory(format!(
            "Directory {:?} does not exist",
            path
        )));
    }

    if !path.is_dir() {
        return Err(FileManagerError::InvalidDirectory(format!(
            "Path {:?} is not a directory",
            path
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
pub fn validate_compress_path(
    dst: &Path,
    method: CompressionMethod,
) -> Result<(), FileManagerError> {
    let valid_extensions: &[&str] = match method {
        CompressionMethod::Gzip => &[".tar.gz", ".tgz"],
        CompressionMethod::Zstd => &[".tar.zst", ".tzst"],
        CompressionMethod::Lz4 => &[".tar.lz4"],
        CompressionMethod::Xz => &[".tar.xz"],
        CompressionMethod::Bzip2 => &[".tar.bz2", ".tbz", ".tbz2"],
    };
    let dst_str = dst.to_str().ok_or(FileManagerError::InvalidInput(
        "Destination path is not valid UTF‑8".into(),
    ))?;

    if !valid_extensions.iter().any(|ext| dst_str.ends_with(ext)) {
        return Err(FileManagerError::InvalidInput(format!(
            "Destination {:?} must have a valid compression extension: {:?}",
            dst, valid_extensions
        )));
    }

    Ok(())
}

/// Validates that a ZIP archive destination uses the portable ZIP extension.
pub fn validate_zip_path(dst: &Path) -> Result<(), FileManagerError> {
    let dst_str = dst.to_str().ok_or(FileManagerError::InvalidInput(
        "Destination path is not valid UTF-8".into(),
    ))?;
    if dst_str.to_lowercase().ends_with(".zip") {
        Ok(())
    } else {
        Err(FileManagerError::InvalidInput(format!(
            "ZIP destinations must use the .zip extension: {dst:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{ensure_not_nested, validate_compress_path, validate_zip_path};
    use crate::file_module::compress::CompressionMethod;
    use crate::test::TestDir;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    #[test]
    fn compression_extensions_match_the_selected_encoder() {
        assert!(
            validate_compress_path(Path::new("backup.tar.gz"), CompressionMethod::Gzip).is_ok()
        );
        assert!(validate_compress_path(Path::new("backup.tgz"), CompressionMethod::Gzip).is_ok());
        assert!(
            validate_compress_path(Path::new("backup.tar.zst"), CompressionMethod::Zstd).is_ok()
        );
        assert!(validate_compress_path(Path::new("backup.tzst"), CompressionMethod::Zstd).is_ok());
        assert!(
            validate_compress_path(Path::new("backup.tar.lz4"), CompressionMethod::Lz4).is_ok()
        );
        assert!(validate_compress_path(Path::new("backup.tar.xz"), CompressionMethod::Xz).is_ok());
        assert!(
            validate_compress_path(Path::new("backup.tar.bz2"), CompressionMethod::Bzip2).is_ok()
        );
        assert!(validate_compress_path(Path::new("backup.tbz"), CompressionMethod::Bzip2).is_ok());
        assert!(validate_compress_path(Path::new("backup.tbz2"), CompressionMethod::Bzip2).is_ok());

        assert!(
            validate_compress_path(Path::new("backup.tar.zst"), CompressionMethod::Gzip).is_err()
        );
        assert!(
            validate_compress_path(Path::new("backup.tar.gz"), CompressionMethod::Zstd).is_err()
        );
        assert!(
            validate_compress_path(Path::new("backup.tar.zst"), CompressionMethod::Lz4).is_err()
        );
        assert!(
            validate_compress_path(Path::new("backup.tar.xz"), CompressionMethod::Lz4).is_err()
        );
        assert!(
            validate_compress_path(Path::new("backup.tar.bz2"), CompressionMethod::Xz).is_err()
        );
    }

    #[test]
    fn zip_extensions_are_validated_separately_from_tar_codecs() {
        assert!(validate_zip_path(Path::new("backup.zip")).is_ok());
        assert!(validate_zip_path(Path::new("backup.ZIP")).is_ok());
        assert!(validate_zip_path(Path::new("backup.tar.gz")).is_err());
        assert!(validate_zip_path(Path::new("backup.zipx")).is_err());
    }

    #[test]
    fn relative_missing_destination_cannot_be_nested_in_source() {
        let root_name = format!(
            ".arkive-nested-check-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::current_dir().unwrap().join(&root_name);
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();

        let relative_source = PathBuf::from(&root_name).join("source");
        let relative_destination = relative_source.join("nested/output");
        let result = ensure_not_nested(&relative_source, &relative_destination);

        fs::remove_dir_all(root).unwrap();
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlink_destination_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("nested-symlink");
        let source = temp.path().join("source");
        let outside = temp.path().join("outside");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink(&outside, &destination).unwrap();

        let result = ensure_not_nested(&source, &destination);

        assert!(result.is_err());
    }
}
