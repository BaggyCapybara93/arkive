use parking_lot::Mutex;
use std::path::PathBuf;

use super::error::FileManagerError;

pub struct FileManager {
    pub file_path: PathBuf,
    pub file_dest: PathBuf,
    pub lock: Mutex<()>,
}

impl FileManager {
    pub fn new(file_path: impl Into<PathBuf>, file_dest: impl Into<PathBuf>) -> Self {
        FileManager { 
            file_path: file_path.into(), 
            file_dest: file_dest.into(),
            lock: Mutex::new(()),
        }
    }

    // Delegates to trash.rs for trash path handling
    pub fn trash_dir() -> Result<PathBuf, FileManagerError> {
        super::trash::trash_dir()
    }
}
