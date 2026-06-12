use std::path::PathBuf;

use crate::file_module::error::FileManagerError;
use crate::settings::Settings;

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

pub fn empty_trash(settings: &Settings) -> Result<(), FileManagerError> {
    let trash = trash_dir()?;

    if trash.exists() {
        std::fs::remove_dir_all(&trash)?;
        std::fs::create_dir_all(&trash)?;
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