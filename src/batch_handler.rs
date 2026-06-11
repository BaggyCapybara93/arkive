use serde::Deserialize;
use serde_json;
use crate::file_manager::{FileManager, FileManagerError};
use crate::settings::Settings;
use std::fs;
use std::sync::Arc;
use std::{
    sync::{mpsc, Mutex},
    thread,
};

//Error Handling
#[derive(thiserror::Error, Debug)]
pub enum BatchError {
    #[error("Failed to read batch file: {0}")]
    Read(#[from] std::io::Error),

    #[error("Failed to parse batch file: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("Worker thread failed: {0}")]
    Worker(String),

    #[error("Thread pool error: {0}")]
    ThreadPool(String),
}

//Multithreading
enum ThreadMessage {
    Job(Job),
    Shutdown,
}

enum ThreadResult {
    Ok,
    Err(String),
}

pub struct Worker {
    pub handle: thread::JoinHandle<()>,
}

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<ThreadMessage>,
    result_receiver: mpsc::Receiver<ThreadResult>,
}

impl ThreadPool {
    pub fn new(size: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<ThreadMessage>();
        let (result_sender, result_receiver) = mpsc::channel::<ThreadResult>();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            let receiver = Arc::clone(&receiver);
            let result_sender = result_sender.clone();

            let handle = thread::spawn(move || {
                loop {
                    let message = match receiver.lock() {
                        Ok(guard) => guard.recv(),
                        Err(e) => {
                            eprintln!("Worker {id} failed to lock receiver: {e}");
                            break;
                        }
                    };

                    match message {
                        Ok(ThreadMessage::Job(job)) => {
                            match job.execute() {
                                Ok(_) => {
                                    let _ = result_sender.send(ThreadResult::Ok);
                                }
                                Err(e) => {
                                    let _ = result_sender.send(ThreadResult::Err(format!("Worker {id} failed to execute job: {e}")));
                                }
                            }
                        }

                        Ok (ThreadMessage::Shutdown) => break,
                        Err(_) => break,
                    }
                }
            });

            workers.push(Worker { handle });
        }

        ThreadPool { workers, sender, result_receiver }
    }

    pub fn join(self) -> Result<(), BatchError> {
        for _ in &self.workers {
            let _ = self.sender.send(ThreadMessage::Shutdown);
        }

        for worker in self.workers {
            worker.handle.join()
                .map_err(|_| BatchError::ThreadPool("Worker thread panicked".into()))?;
        }

        while let Ok(result) = self.result_receiver.recv() {
            if let ThreadResult::Err(e) = result {
                return Err(BatchError::Worker(e));
            }
        }

        Ok(())
    }

    pub fn add_job(&self, job: Job) -> Result<(), BatchError> {
        self.sender.send(ThreadMessage::Job(job))
                .map_err(|e| BatchError::ThreadPool(e.to_string()))
    }

}


#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum WorkType {
    Move,
    Copy,
    Compress,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BatchCompressionMethod {
    Gzip,
    Zstd,
}

impl From<BatchCompressionMethod> for crate::file_manager::compress::CompressionMethod {
    fn from(batch_method: BatchCompressionMethod) -> Self {
        match batch_method {
            BatchCompressionMethod::Gzip => crate::file_manager::compress::CompressionMethod::Gzip,
            BatchCompressionMethod::Zstd => crate::file_manager::compress::CompressionMethod::Zstd,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Job {
    pub work_type: WorkType,
    pub source: String,
    pub destination: Option<String>,
    pub recursive: Option<bool>,
    pub to_trash: Option<bool>,
    pub compression_method: Option<BatchCompressionMethod>,
    #[serde(skip)]
    pub settings: Option<Arc<Settings>>,
}

impl Job {
    pub fn execute(&self) -> Result<(), FileManagerError> {
        let settings = self.settings.as_ref()
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
                    fm.copy_path(true)?;
                    fm.delete_path(self.source.clone(), true, false)?;
                } else {
                    fm.move_path()?;
                }
            }
            WorkType::Copy => fm.copy_path(recursive)?,
            WorkType::Compress => {
                let compression_method = self.compression_method
                    .as_ref()
                    .map(|m| m.clone().into())
                    .or_else(|| self.settings.as_ref().map(|s| s.compression_method));
                let method = compression_method.unwrap_or(settings.compression_method);
                fm.compress_path(method)?;
            }
        }

        Ok(())
    }
}

pub struct BatchHandler {
    pub commands: Vec<Job>,
    pub settings: Arc<Settings>,
}

impl BatchHandler {
    pub fn new(commands: Vec<Job>, settings: &Settings) -> Self {
        let settings = Arc::new(settings.clone());
        let commands = commands.into_iter().map(|mut job| {
            job.settings = Some(settings.clone());
            job
        }).collect();
        BatchHandler { commands, settings }
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
            return Ok(BatchHandler { commands, settings: Arc::new(settings.clone()) });
        }
        
        // Fall back to plain Vec<Job> (old format)
        let commands: Vec<Job> = serde_json::from_str(&batch_content)?;
        Ok(BatchHandler::new(commands, settings))
    }

    pub fn run(&self) -> Result<(), BatchError> {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);

        // Use half of the available threads to prevent overwhelming the system
        let max_threads = std::cmp::max(1, threads / 2);

        //Prevent adding more threads than jobs in batch file
        let job_count = self.commands.len();
        let worker_count = std::cmp::min(max_threads, job_count.max(1));

        let pool = ThreadPool::new(worker_count);

        for job in &self.commands {
            println!("Running job: {:?} -> {:?}", job.work_type, job.destination);
            pool.add_job(job.clone())?;
        }
        pool.join()
    }
}

#[derive(Debug, Deserialize)]
pub struct BatchFile {
    #[serde(rename = "operations")]
    pub operations: Vec<Job>,
}