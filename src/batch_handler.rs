use serde::Deserialize;
use serde_json;
use crate::file_manager::FileManager;
use std::fs;
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
};

//Multithreading
enum ThreadMessage {
    Job(Job),
    Shutdown,
}

pub struct Worker {
    pub id: usize,
    pub handle: thread::JoinHandle<()>,
}

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<ThreadMessage>,
}

impl ThreadPool {
    pub fn new(size: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<ThreadMessage>();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            let receiver = Arc::clone(&receiver);

            let handle = thread::spawn(move || {
                loop {
                    let message = receiver.lock().unwrap().recv();
                    match message {
                        Ok(ThreadMessage::Job(job)) => {
                            println!("Worker {id} executing job");
                            let _ = job.execute();
                        }
                        Ok(ThreadMessage::Shutdown) => {
                            println!("Worker {id} shutting down");
                            break;
                        }
                        Err(_) => break,
                    }
                }
            });

            workers.push(Worker { id, handle });
        }

        ThreadPool { workers, sender }
    }

    pub fn join(self) {
        for _ in &self.workers {
            self.sender.send(ThreadMessage::Shutdown).unwrap();
        }

        for worker in self.workers {
            let _ = worker.handle.join();
        }
    }

    pub fn add_job(&self, job: Job) {
        self.sender
            .send(ThreadMessage::Job(job))
            .expect("Failed to send job to thread pool");
    }

}


#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum WorkType {
    Move,
    Copy,
    Compress,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Job {
    pub work_type: WorkType,
    pub source: String,
    pub destination: Option<String>,
    pub recursive: Option<bool>,
}

impl Job {
    pub fn execute(&self) -> std::io::Result<()> {
        let recursive = self.recursive.unwrap_or(false);
        let dest = self.destination.clone().unwrap_or_else(|| self.source.clone());
        let fm = FileManager::new(self.source.clone(), dest);

        match self.work_type {
            WorkType::Move => {
                if recursive {
                    fm.copy_path(true)?;
                    fm.delete_path(true)?;
                } else {
                    fm.move_path()?;
                }
            }
            WorkType::Copy => fm.copy_path(recursive)?,
            WorkType::Compress => fm.compress_path()?,
        }
        Ok(())
    }
}

pub struct BatchHandler {
    pub commands: Vec<Job>,
}

impl BatchHandler {
    pub fn new(commands: Vec<Job>) -> Self{
        BatchHandler { commands }
    }

    pub fn run(&self) {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);

        // Use half of the available threads to prevent overwhelming the system
        let max_threads = std::cmp::max(1, threads / 2);

        //Prevent adding more threads than jobs in batch file
        let job_count = self.commands.len();
        let worker_count = std::cmp::min(max_threads, job_count.max(1));

        let pool = ThreadPool::new(worker_count);

        for job in &self.commands {
            println!("Running job: {:?} -> {:?}", job.work_type, job.destination);
            pool.add_job(job.clone());
        }
        pool.join();
    }

    pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let data = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read batch file {}: {}", path, e))?;

        let commands: Vec<Job> = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse batch file {}: {}", path, e))?;

        Ok(Self::new(commands))
    }
}