use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use parking_lot::{Condvar, Mutex};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use crate::settings::Settings;

struct PathLockRegistry {
    active: Mutex<HashSet<PathBuf>>,
    available: Condvar,
}

static PATH_LOCKS: LazyLock<PathLockRegistry> = LazyLock::new(|| PathLockRegistry {
    active: Mutex::new(HashSet::new()),
    available: Condvar::new(),
});

/// Owns reservations in the global path registry. Dropping it wakes any
/// operations waiting on the same path or one of its ancestors/descendants.
pub struct OperationGuard {
    paths: Vec<PathBuf>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut active = PATH_LOCKS.active.lock();
        for path in &self.paths {
            active.remove(path);
        }
        PATH_LOCKS.available.notify_all();
    }
}

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

    /// Reserve this operation's source and destination. Operations on disjoint
    /// paths can proceed concurrently; exact matches and ancestor/descendant
    /// overlaps wait until the earlier operation completes.
    pub fn acquire_lock(&self) -> OperationGuard {
        Self::acquire_paths([self.file_path.as_path(), self.file_dest.as_path()])
    }

    pub(crate) fn acquire_paths<'p>(paths: impl IntoIterator<Item = &'p Path>) -> OperationGuard {
        let mut paths = paths
            .into_iter()
            .filter(|path| !path.as_os_str().is_empty())
            .map(normalize_lock_path)
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();

        let mut active = PATH_LOCKS.active.lock();
        while paths
            .iter()
            .any(|requested| active.iter().any(|held| paths_overlap(requested, held)))
        {
            PATH_LOCKS.available.wait(&mut active);
        }
        active.extend(paths.iter().cloned());
        OperationGuard { paths }
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

fn normalize_lock_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let mut unresolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut suffix = Vec::new();
    while !unresolved.exists() {
        if let Some(name) = unresolved.file_name() {
            suffix.push(name.to_os_string());
        }
        if !unresolved.pop() {
            break;
        }
    }
    let mut absolute = std::fs::canonicalize(&unresolved).unwrap_or(unresolved);
    for component in suffix.into_iter().rev() {
        absolute.push(component);
    }

    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

#[cfg(test)]
mod tests {
    use super::FileManager;
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn progress_bars_are_only_created_for_multi_item_operations() {
        assert!(FileManager::maybe_create_progress_bar(1, "test").is_none());
        assert!(FileManager::maybe_create_progress_bar(2, "test").is_some());
    }

    #[test]
    fn disjoint_paths_can_be_locked_concurrently() {
        let first = FileManager::acquire_paths([Path::new("/tmp/arkive-lock-a")]);
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _second = FileManager::acquire_paths([Path::new("/tmp/arkive-lock-b")]);
            sender.send(()).unwrap();
        });

        receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(first);
        worker.join().unwrap();
    }

    #[test]
    fn overlapping_paths_wait_for_the_existing_operation() {
        let first = FileManager::acquire_paths([Path::new("/tmp/arkive-lock-root")]);
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _second = FileManager::acquire_paths([Path::new("/tmp/arkive-lock-root/child")]);
            sender.send(()).unwrap();
        });

        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
        drop(first);
        receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }
}
