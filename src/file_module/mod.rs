pub mod cleanup;
pub mod compress;
pub mod copy;
pub mod dedup;
pub mod deploy;
pub mod error;
pub mod manager;
pub mod metadata;
pub mod ops;
pub mod remove;
pub mod rename;
pub mod trash;

use std::path::Path;
use std::path::PathBuf;

/// Add timestamp to the front of a path
pub fn add_timestamp_to_path(path: &Path) -> Result<PathBuf, FileManagerError> {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let path_str = path
        .to_str()
        .ok_or_else(|| FileManagerError::InvalidInput("Path is not valid UTF-8".into()))?;

    // Split the path into directory and filename
    let (dir, file) = path_str
        .rsplit_once(|c| c == '/' || c == '\\')
        .map(|(d, f)| (d, Some(f)))
        .unwrap_or((path_str, Some(path_str)));

    let new_filename = format!("{}_{}", timestamp, file.unwrap_or(""));

    Ok(PathBuf::from(format!("{}/{}", dir, new_filename)))
}

pub use error::FileManagerError;
pub use manager::FileManager;
