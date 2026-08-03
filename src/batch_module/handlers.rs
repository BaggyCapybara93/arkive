use std::sync::Arc;
use serde_json;
use crate::settings::Settings;
use std::fs;

use crate::batch_module::{Job, BatchError};
use crate::batch_module::job::BatchCompressionMethod;
use crate::batch_module::thread_pool::ThreadPool;
use crate::batch_module::file::BatchFile;


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
        let commands = commands.into_iter().map(|mut job| {
            job.settings = Some(settings.clone());
            job
        }).collect();
        BatchHandler { commands }
    }

    pub fn from_file(path: &str, settings: &Settings) -> Result<Self, BatchError> {
        let batch_content = fs::read_to_string(path)?;
        
        // Try parsing as BatchFile first (new format with "operations" key)
        if let Ok(batch) = serde_json::from_str::<BatchFile>(&batch_content) {
            let commands = batch.operations
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
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let progress = if !self.commands.is_empty() {
            Some(indicatif::ProgressBar::new(self.commands.len() as u64))
        } else {
            None
        };

        if let Some(bar) = progress.as_ref() {
            bar.set_draw_target(indicatif::ProgressDrawTarget::stderr());
            bar.enable_steady_tick(std::time::Duration::from_millis(120));
            bar.set_message("Running batch jobs");
            bar.set_style(indicatif::ProgressStyle::with_template("{msg} [{bar:40.cyan/blue}] {pos}/{len}").unwrap().progress_chars("=>-"));
        }

        // Use half of the available threads to prevent overwhelming the system
        let max_threads = std::cmp::max(1, threads / 2);

        //Prevent adding more threads than jobs in batch file
        let job_count = self.commands.len();
        let worker_count = std::cmp::min(max_threads, job_count.max(1));

        let pool = ThreadPool::new(worker_count);

        for job in &self.commands {
            println!("Running job: {:?} -> {:?}", job.work_type, job.destination);
            pool.add_job(job.clone())?;
            if let Some(bar) = progress.as_ref() {
                bar.inc(1);
            }
        }
        let result = pool.join();
        if let Some(bar) = progress {
            bar.finish_with_message("Batch complete");
        }
        result
    }
}
