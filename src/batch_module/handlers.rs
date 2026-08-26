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
        }
    }
}

pub struct BatchHandler {
    pub commands: Vec<Job>,
}

impl BatchHandler {
    pub fn new(commands: Vec<Job>, settings: &Settings) -> Self {
        let settings = Arc::new(settings.clone());
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
                    job.settings = Some(Arc::new(settings.clone()));
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

        for job in &self.commands {
            println!("Running job: {:?} -> {:?}", job.work_type, job.destination);
            pool.add_job(job.clone())?;
        }

        // Wait for all jobs to complete
        let result = pool.join();

        if let Some(bar) = progress {
            bar.finish_with_message("Batch complete");
        }

        result
    }
}
