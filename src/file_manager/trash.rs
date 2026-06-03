use std::path::PathBuf;

use crate::file_manager::error::FileManagerError;

/// Get the path to the arkive trash directory.
/// Creates the directory if it doesn't exist.
pub fn trash_dir() -> Result<PathBuf, FileManagerError> {
    let cwd = std::env::current_dir()?;
    let trash = cwd.join("arkive_trash");

    if !trash.exists() {
        std::fs::create_dir_all(&trash)?;
    }

    Ok(trash)
}
