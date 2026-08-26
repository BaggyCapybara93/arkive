use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::time::Duration;

use crate::settings::Settings;

static GLOBAL_FILE_OPERATION_LOCK: Mutex<()> = Mutex::new(());

pub struct FileManager<'a> {
    pub file_path: PathBuf,
    pub file_dest: PathBuf,
    pub settings: &'a Settings,
}

impl<'a> FileManager<'a> {
    pub fn new(
        file_path: impl Into<PathBuf>,
        file_dest: impl Into<PathBuf>,
        settings: &'a Settings,
    ) -> Self {
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

    pub(crate) fn create_progress_bar(len: u64, message: impl Into<String>) -> ProgressBar {
        let bar = ProgressBar::new(len);
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.enable_steady_tick(Duration::from_millis(120));
        bar.set_message(message.into());
        bar.set_style(
            ProgressStyle::with_template("{msg} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("=>-"),
        );
        bar
    }

    pub(crate) fn create_spinner(message: impl Into<String>) -> ProgressBar {
        let bar = ProgressBar::new_spinner();
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.enable_steady_tick(Duration::from_millis(120));
        bar.set_message(message.into());
        bar.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
        bar
    }

    pub(crate) fn maybe_create_progress_bar(
        len: u64,
        message: impl Into<String>,
    ) -> Option<ProgressBar> {
        if len <= 1 {
            None
        } else {
            Some(Self::create_progress_bar(len, message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FileManager;

    #[test]
    fn progress_bars_are_only_created_for_multi_item_operations() {
        assert!(FileManager::maybe_create_progress_bar(1, "test").is_none());
        assert!(FileManager::maybe_create_progress_bar(2, "test").is_some());
    }
}
