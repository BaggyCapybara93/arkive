use crate::file_module::cleanup::CleanupOptions;
use crate::file_module::compress::ArchiveFormat;
use crate::file_module::{FileManager, FileManagerError};
use crate::settings::Settings;
use indicatif::ProgressBar;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum WorkType {
    Move,
    Copy,
    Compress,
    Rename,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BatchCompressionMethod {
    Gzip,
    Zstd,
    Lz4,
    Xz,
    Bzip2,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BatchArchiveFormat {
    Tar,
    Zip,
}

/// Explicit cleanup actions to run after a successful batch job.
///
/// Cleanup is deliberately an object rather than a boolean: a bare `true`
/// does not say whether it is safe to empty the trash, remove duplicates, or
/// remove empty directories.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct BatchCleanup {
    /// Directory inspected by the directory-scoped cleanup actions.
    pub path: Option<String>,
    #[serde(default)]
    pub empty_trash: bool,
    #[serde(default)]
    pub deduplicate: bool,
    #[serde(default)]
    pub scan_unused: bool,
    #[serde(default)]
    pub scan_empty_dirs: bool,
}

impl BatchCleanup {
    fn options(&self) -> Result<CleanupOptions, FileManagerError> {
        let options = CleanupOptions {
            empty_trash: self.empty_trash,
            deduplicate: self.deduplicate,
            scan_unused: self.scan_unused,
            scan_empty_dirs: self.scan_empty_dirs,
        };
        if !options.empty_trash
            && !options.deduplicate
            && !options.scan_unused
            && !options.scan_empty_dirs
        {
            return Err(FileManagerError::InvalidInput(
                "Batch cleanup must enable at least one cleanup action".into(),
            ));
        }
        if self.path.is_none()
            && (options.deduplicate || options.scan_unused || options.scan_empty_dirs)
        {
            return Err(FileManagerError::InvalidInput(
                "Batch cleanup needs a path for deduplicate, scan_unused, or scan_empty_dirs"
                    .into(),
            ));
        }
        Ok(options)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Job {
    pub work_type: WorkType,
    pub source: String,
    pub destination: Option<String>,
    pub recursive: Option<bool>,
    pub cleanup: Option<BatchCleanup>,
    pub compression_method: Option<BatchCompressionMethod>,
    pub archive_format: Option<BatchArchiveFormat>,
    pub timestamp: Option<bool>, // Add timestamp prefix to destination filename
    #[serde(skip)]
    pub settings: Option<Arc<Settings>>,
}

impl Job {
    pub(crate) fn validate(&self) -> Result<(), FileManagerError> {
        if let Some(cleanup) = &self.cleanup {
            cleanup.options()?;
        }
        Ok(())
    }

    /// `progress` is the shared batch-level bar, passed in by whichever worker
    /// thread picked this job up. It's incremented only after the job (and any
    /// cleanup) truly succeeds, so the bar reflects real completions.
    pub fn execute(&self, progress: Option<&ProgressBar>) -> Result<(), FileManagerError> {
        self.validate()?;
        let settings = self
            .settings
            .as_ref()
            .ok_or_else(|| FileManagerError::InvalidInput("Settings not provided".to_string()))?;
        let recursive = self.recursive.unwrap_or(settings.recursive);
        let dest = match &self.destination {
            Some(dest) => dest.clone(),
            None => self.source.clone(),
        };
        let fm = FileManager::new(self.source.clone(), dest, settings);

        match self.work_type {
            WorkType::Move => {
                if recursive {
                    let add_timestamp = self.timestamp.unwrap_or(settings.use_timestamp);
                    let copied_path = fm.copy_path(true, add_timestamp)?;
                    // Only delete source if copy succeeded (destination exists)
                    if copied_path.exists() {
                        fm.delete_path(self.source.clone(), true, false)?;
                    } else {
                        return Err(FileManagerError::InvalidInput(
                            "Recursive move failed: destination not created".to_string(),
                        ));
                    }
                } else {
                    fm.move_path()?;
                }
            }
            WorkType::Copy => {
                let add_timestamp = self.timestamp.unwrap_or(settings.use_timestamp);
                fm.copy_path(recursive, add_timestamp)?;
            }
            WorkType::Compress => {
                let archive_format = self
                    .archive_format
                    .as_ref()
                    .map(|format| match format {
                        BatchArchiveFormat::Tar => ArchiveFormat::Tar,
                        BatchArchiveFormat::Zip => ArchiveFormat::Zip,
                    })
                    .unwrap_or_default();
                if matches!(archive_format, ArchiveFormat::Zip) {
                    if self.compression_method.is_some() {
                        return Err(FileManagerError::InvalidInput(
                            "compression_method applies only to tar batch archives".into(),
                        ));
                    }
                    let add_timestamp = self.timestamp.unwrap_or(settings.use_timestamp);
                    fm.compress_zip_path(add_timestamp)?;
                } else {
                    let compression_method = self
                        .compression_method
                        .as_ref()
                        .map(|m| m.clone().into())
                        .or_else(|| self.settings.as_ref().map(|s| s.compression_method));
                    let method = compression_method.unwrap_or(settings.compression_method);
                    let add_timestamp = self.timestamp.unwrap_or(settings.use_timestamp);
                    fm.compress_path(method, add_timestamp)?;
                }
            }
            WorkType::Rename => fm.rename_path()?,
        }
        if let Some(cleanup) = &self.cleanup {
            let cleanup_options = cleanup.options()?;
            let cleanup_path = cleanup.path.as_deref().unwrap_or("");
            let cleanup_manager = FileManager::new(cleanup_path, "", settings.as_ref());
            let _guard = cleanup_manager.acquire_lock();
            cleanup_manager.cleanup(cleanup_options)?;
        }

        // Advance the bar by one tick for this specific completed job.
        // inc(1) is safe to call concurrently from multiple worker threads.
        if let Some(bar) = progress {
            bar.inc(1);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{BatchArchiveFormat, BatchCleanup, Job, WorkType};
    use crate::settings::Settings;
    use crate::test::TestDir;
    use std::sync::Arc;

    #[test]
    fn timestamped_recursive_move_deletes_source_after_copying_actual_destination() {
        let temp = TestDir::new("batch-timestamped-move");
        let source = temp.path().join("source");
        let destination = temp.path().join("backup");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("file.txt"), b"move me").unwrap();

        let settings = Settings {
            use_timestamp: true,
            ..Settings::default()
        };
        let job = Job {
            work_type: WorkType::Move,
            source: source.to_string_lossy().into_owned(),
            destination: Some(destination.to_string_lossy().into_owned()),
            recursive: Some(true),
            cleanup: None,
            compression_method: None,
            archive_format: None,
            timestamp: Some(true),
            settings: Some(Arc::new(settings)),
        };

        job.execute(None).unwrap();

        assert!(!source.exists());
        let timestamped_destinations: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("20"))
            })
            .collect();
        assert_eq!(timestamped_destinations.len(), 1);
        assert_eq!(
            std::fs::read(timestamped_destinations[0].join("file.txt")).unwrap(),
            b"move me"
        );
    }

    #[test]
    fn zip_batch_job_uses_the_zip_archive_format() {
        let temp = TestDir::new("batch-zip-compress");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("backup.zip");
        std::fs::write(&source, b"batch archive me").unwrap();

        let job = Job {
            work_type: WorkType::Compress,
            source: source.to_string_lossy().into_owned(),
            destination: Some(destination.to_string_lossy().into_owned()),
            recursive: None,
            cleanup: None,
            compression_method: None,
            archive_format: Some(BatchArchiveFormat::Zip),
            timestamp: None,
            settings: Some(Arc::new(Settings::default())),
        };

        job.execute(None).unwrap();

        let file = std::fs::File::open(destination).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut entry = archive.by_name("source.txt").unwrap();
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
        assert_eq!(contents, b"batch archive me");
    }

    #[test]
    fn cleanup_runs_only_the_requested_actions_on_its_explicit_path() {
        let temp = TestDir::new("batch-cleanup");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");
        let cleanup_root = temp.path().join("cleanup-root");
        let empty_directory = cleanup_root.join("empty");
        std::fs::write(&source, b"copy before cleanup").unwrap();
        std::fs::create_dir_all(&empty_directory).unwrap();

        let job = Job {
            work_type: WorkType::Copy,
            source: source.to_string_lossy().into_owned(),
            destination: Some(destination.to_string_lossy().into_owned()),
            recursive: None,
            cleanup: Some(BatchCleanup {
                path: Some(cleanup_root.to_string_lossy().into_owned()),
                scan_empty_dirs: true,
                ..BatchCleanup::default()
            }),
            compression_method: None,
            archive_format: None,
            timestamp: None,
            settings: Some(Arc::new(Settings::default())),
        };

        job.execute(None).unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), b"copy before cleanup");
        assert!(!empty_directory.exists());
    }

    #[test]
    fn cleanup_rejects_an_empty_action_set_before_it_runs() {
        let cleanup = BatchCleanup::default();
        assert!(cleanup.options().is_err());
    }

    #[test]
    fn invalid_cleanup_prevents_the_job_operation() {
        let temp = TestDir::new("batch-invalid-cleanup");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");
        std::fs::write(&source, b"do not copy").unwrap();
        let job = Job {
            work_type: WorkType::Copy,
            source: source.to_string_lossy().into_owned(),
            destination: Some(destination.to_string_lossy().into_owned()),
            recursive: None,
            cleanup: Some(BatchCleanup::default()),
            compression_method: None,
            archive_format: None,
            timestamp: None,
            settings: Some(Arc::new(Settings::default())),
        };

        assert!(job.execute(None).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn directory_cleanup_requires_an_explicit_path() {
        let cleanup = BatchCleanup {
            scan_empty_dirs: true,
            ..BatchCleanup::default()
        };
        assert!(cleanup.options().is_err());
    }
}
