use std::fs;
use std::path::Path;

use crate::file_manager::error::FileManagerError;
use crate::file_validation::handlers::ensure_not_nested;

/// Recursively copy a directory and its contents to the destination.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), FileManagerError> {
    if src.is_dir() {
        crate::file_validation::handlers::valid_directory(src)?;
    }

    ensure_not_nested(src, dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}
