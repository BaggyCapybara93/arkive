use crate::batch_module::{BatchError, Job};
use indicatif::ProgressBar;
use std::sync::Arc;
use std::{
    sync::{Mutex, mpsc},
    thread,
};

pub enum ThreadMessage {
    Job(Job),
    Shutdown,
}

pub enum ThreadResult {
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
    /// `progress` is the shared batch-level bar (or None if there are no jobs).
    /// It's captured once here and cloned into every worker thread, so each
    /// worker can call `.inc(1)` on the SAME underlying bar the instant its
    /// own job finishes.
    pub fn new(size: usize, progress: Option<ProgressBar>) -> Self {
        let (sender, receiver) = mpsc::channel::<ThreadMessage>();
        let (result_sender, result_receiver) = mpsc::channel::<ThreadResult>();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            let receiver = Arc::clone(&receiver);
            let result_sender = result_sender.clone();
            // ProgressBar is internally Arc-backed, so this clone is cheap and
            // all workers end up incrementing the same visible bar.
            let progress = progress.clone();

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
                        Ok(ThreadMessage::Job(job)) => match job.execute(progress.as_ref()) {
                            Ok(_) => {
                                let _ = result_sender.send(ThreadResult::Ok);
                            }
                            Err(e) => {
                                let _ = result_sender.send(ThreadResult::Err(format!(
                                    "Worker {id} failed to execute job: {e}"
                                )));
                            }
                        },

                        Ok(ThreadMessage::Shutdown) => break,
                        Err(_) => break,
                    }
                }
            });

            workers.push(Worker { handle });
        }

        ThreadPool {
            workers,
            sender,
            result_receiver,
        }
    }

    pub fn join(self) -> Result<(), BatchError> {
        for _ in &self.workers {
            let _ = self.sender.send(ThreadMessage::Shutdown);
        }

        for worker in self.workers {
            worker
                .handle
                .join()
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
        self.sender
            .send(ThreadMessage::Job(job))
            .map_err(|e| BatchError::ThreadPool(e.to_string()))
    }
}
