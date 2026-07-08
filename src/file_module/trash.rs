use std::path::PathBuf;
use std::fs;

use crate::file_module::error::FileManagerError;
use crate::settings::Settings;

/// Get the path to the arkive trash directory.
/// Creates the directory if it doesn't exist.
pub fn trash_dir() -> Result<PathBuf, FileManagerError> {
    let trash = if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join("arkive_trash")
    } else {
        std::env::current_dir()?.join("arkive_trash")
    };

    if !trash.exists() {
        std::fs::create_dir_all(&trash)?;
    }

    Ok(trash)
}

pub fn empty_trash(settings: &Settings) -> Result<(), FileManagerError> {
    let trash = trash_dir()?;

    if !trash.is_dir() {
        return Err(FileManagerError::InvalidDirectory(format!(
            "Trash path is not a directory: {:?}", trash
        )));
    }

    // Remove contents
    for entry in fs::read_dir(&trash)? {
        let entry = entry?;
        let path = entry.path();

        // Reject symlinks
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            return Err(FileManagerError::PermissionDenied(
                format!("Symlink found in trash: {:?}", path)
            ));
        }

        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }

    if settings.verbose {
        println!("Trash emptied");
    }

    Ok(())
}

pub fn list_trash(_settings: &Settings) -> Result<(), FileManagerError> {
    let trash = trash_dir()?;

    for entry in std::fs::read_dir(trash)? {
        let entry = entry?;
        println!("{:?}", entry.path());
    }
    Ok(())
}