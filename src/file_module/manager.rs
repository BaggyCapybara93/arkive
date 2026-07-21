use parking_lot::Mutex;
use std::path::PathBuf;

use crate::settings::Settings;

pub struct FileManager<'a> {
    pub file_path: PathBuf,
    pub file_dest: PathBuf,
    lock: Mutex<()>,
    pub settings: &'a Settings,
}

impl<'a> FileManager<'a> {
    pub fn new(file_path: impl Into<PathBuf>, file_dest: impl Into<PathBuf>, settings: &'a Settings) -> Self {
        FileManager { 
            file_path: file_path.into(), 
            file_dest: file_dest.into(),
            lock: Mutex::new(()),
            settings,
        }
    }

    /// Acquires an exclusive lock on the FileManager.
    /// Call this method before performing operations that require mutual exclusion.
    pub fn acquire_lock(&self) -> parking_lot::MutexGuard<'_, ()> {
        self.lock.lock()
    }
}
