use crate::settings::Settings;
use serde_json;
use std::fs;
use std::sync::Arc;

use crate::batch_module::file::BatchFile;
use crate::batch_module::job::BatchCompressionMethod;
use crate::batch_module::thread_pool::ThreadPool;
use crate::batch_module::{BatchError, Job};

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

impl From<BatchCompressionMethod> for crate::file_module::compress::CompressionMethod {
    fn from(batch_method: BatchCompressionMethod) -> Self {
        match batch_method {
            BatchCompressionMethod::Gzip => crate::file_module::compress::CompressionMethod::Gzip,
            BatchCompressionMethod::Zstd => crate::file_module::compress::CompressionMethod::Zstd,
            BatchCompressionMethod::Lz4 => crate::file_module::compress::CompressionMethod::Lz4,
            BatchCompressionMethod::Xz => crate::file_module::compress::CompressionMethod::Xz,
            BatchCompressionMethod::Bzip2 => crate::file_module::compress::CompressionMethod::Bzip2,
        }
    }
}

pub struct BatchHandler {
    pub commands: Vec<Job>,
}

impl BatchHandler {
    pub fn new(commands: Vec<Job>, settings: &Settings) -> Self {
        let settings = Arc::new(*settings);
        let commands = commands
            .into_iter()
            .map(|mut job| {
                job.settings = Some(settings.clone());
                job
            })
            .collect();
        BatchHandler { commands }
    }

    pub fn from_file(path: &str, settings: &Settings) -> Result<Self, BatchError> {
        let batch_content = fs::read_to_string(path)?;
        // Try parsing as BatchFile first (new format with "operations" key)
        if let Ok(batch) = serde_json::from_str::<BatchFile>(&batch_content) {
            let commands = batch
                .operations
                .into_iter()
                .map(|mut job| {
                    job.settings = Some(Arc::new(*settings));
                    job
                })
                .collect();
            return Ok(BatchHandler { commands });
        }
        // Fall back to plain Vec<Job> (old format)
        let commands: Vec<Job> = serde_json::from_str(&batch_content)?;
        Ok(BatchHandler::new(commands, settings))
    }

    pub fn run(&self) -> Result<(), BatchError> {
        for (index, job) in self.commands.iter().enumerate() {
            job.validate().map_err(|source| BatchError::Job {
                index: index + 1,
                source,
            })?;
        }

        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        // MultiProgress coordinates the draw target so multiple bars (this one,
        // plus any per-file bar elsewhere in the pipeline) render as stacked
        // lines instead of overwriting each other.
        let multi = MultiProgress::new();

        let progress = if !self.commands.is_empty() {
            let bar = multi.add(ProgressBar::new(self.commands.len() as u64));
            bar.set_draw_target(ProgressDrawTarget::stderr());
            bar.set_message("Running batch jobs");
            bar.set_style(
                ProgressStyle::with_template("{msg} [{bar:40.cyan/blue}] {pos}/{len}")
                    .unwrap()
                    .progress_chars("=>-"),
            );
            Some(bar)
        } else {
            None
        };

        // Use half of the available threads to prevent overwhelming the system
        let max_threads = std::cmp::max(1, threads / 2);

        // Prevent adding more threads than jobs in batch file
        let job_count = self.commands.len();
        let worker_count = std::cmp::min(max_threads, job_count.max(1));

        // Hand a clone of the bar to the pool; each worker thread will clone it
        // again internally so it can call .inc(1) when its own job finishes.
        let pool = ThreadPool::new(worker_count, progress.clone());

        let mut submission_error = None;
        for (index, job) in self.commands.iter().enumerate() {
            println!(
                "Running job {}: {:?} -> {:?}",
                index + 1,
                job.work_type,
                job.destination
            );
            if let Err(error) = pool.add_job(index + 1, job.clone()) {
                submission_error = Some(error);
                break;
            }
        }

        // Wait for all jobs to complete
        let result = match (submission_error, pool.join()) {
            (_, Err(error)) => Err(error),
            (Some(error), Ok(())) => Err(error),
            (None, Ok(())) => Ok(()),
        };

        if let Some(bar) = progress {
            if result.is_ok() {
                bar.finish_with_message("Batch complete");
            } else {
                bar.abandon_with_message("Batch failed");
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::BatchHandler;
    use crate::batch_module::job::{BatchCleanup, WorkType};
    use crate::batch_module::{BatchError, Job};
    use crate::file_module::FileManagerError;
    use crate::settings::Settings;
    use crate::test::TestDir;
    use std::error::Error;
    use std::path::Path;

    fn copy_job(source: &Path, destination: &Path) -> Job {
        Job {
            work_type: WorkType::Copy,
            source: source.to_string_lossy().into_owned(),
            destination: Some(destination.to_string_lossy().into_owned()),
            recursive: None,
            cleanup: None,
            compression_method: None,
            archive_format: None,
            timestamp: None,
            settings: None,
        }
    }

    #[test]
    fn validation_error_identifies_job_and_preserves_cause() {
        let temp = TestDir::new("batch-validation-error");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");
        let first = copy_job(&source, &destination);
        let mut second = copy_job(&source, &destination);
        second.cleanup = Some(BatchCleanup::default());
        let handler = BatchHandler::new(vec![first, second], &Settings::default());

        let error = handler.run().unwrap_err();

        assert!(matches!(&error, BatchError::Job { index: 2, .. }));
        assert!(
            Error::source(&error)
                .unwrap()
                .downcast_ref::<FileManagerError>()
                .is_some()
        );
        assert!(!destination.exists());
    }

    #[test]
    fn execution_error_identifies_job_and_preserves_cause() {
        let temp = TestDir::new("batch-execution-error");
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");
        let missing = temp.path().join("missing.txt");
        let other_destination = temp.path().join("other.txt");
        std::fs::write(&source, b"copied").unwrap();
        let handler = BatchHandler::new(
            vec![
                copy_job(&source, &destination),
                copy_job(&missing, &other_destination),
            ],
            &Settings::default(),
        );

        let error = handler.run().unwrap_err();

        assert!(matches!(&error, BatchError::Job { index: 2, .. }));
        assert!(
            Error::source(&error)
                .unwrap()
                .downcast_ref::<FileManagerError>()
                .is_some()
        );
        assert_eq!(std::fs::read(destination).unwrap(), b"copied");
    }

    #[test]
    fn multiple_failures_report_the_lowest_numbered_job() {
        let temp = TestDir::new("batch-multiple-errors");
        let handler = BatchHandler::new(
            vec![
                copy_job(&temp.path().join("missing-1"), &temp.path().join("copy-1")),
                copy_job(&temp.path().join("missing-2"), &temp.path().join("copy-2")),
            ],
            &Settings::default(),
        );

        let error = handler.run().unwrap_err();

        assert!(matches!(error, BatchError::Job { index: 1, .. }));
    }
}
