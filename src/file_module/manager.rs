use parking_lot::Mutex;
use std::path::PathBuf;

use crate::settings::Settings;

static GLOBAL_FILE_OPERATION_LOCK: Mutex<()> = Mutex::new(());

pub struct FileManager<'a> {
    pub file_path: PathBuf,
    pub file_dest: PathBuf,
    pub settings: &'a Settings,
}

impl<'a> FileManager<'a> {
    pub fn new(file_path: impl Into<PathBuf>, file_dest: impl Into<PathBuf>, settings: &'a Settings) -> Self {
        FileManager { 
            file_path: file_path.into(), 
            file_dest: file_dest.into(),
            settings,
        }
    }

    /// Acquires an exclusive lock for file operations.
    /// This is shared across all FileManager instances so concurrent operations
    /// targeting the same paths cannot race.
    pub fn acquire_lock(&self) -> parking_lot::MutexGuard<'static, ()> {
        GLOBAL_FILE_OPERATION_LOCK.lock()
    }
}
